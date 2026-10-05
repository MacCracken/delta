//! Git transport protocol helpers.
//!
//! Implements the server side of the git smart HTTP transport:
//! - `GET /info/refs?service=git-upload-pack` — ref advertisement for clone/fetch
//! - `GET /info/refs?service=git-receive-pack` — ref advertisement for push
//! - `POST /git-upload-pack` — pack negotiation and data transfer (clone/fetch)
//! - `POST /git-receive-pack` — receive pushed data
//!
//! and the processes behind the SSH transport ([`spawn_service_session`]).

use std::path::Path;
use std::process::Stdio;
use tokio::process::{Child, Command};

use delta_core::{DeltaError, Result};

/// Run `git-upload-pack --advertise-refs` or `git-receive-pack --advertise-refs`
/// for the info/refs endpoint.
pub async fn advertise_refs(repo_path: &Path, service: &str) -> Result<Vec<u8>> {
    let advertisement = ref_advertisement(repo_path, service).await?;

    // Build the smart HTTP response:
    // First line: pkt-line with "# service=git-upload-pack\n"
    // Then: flush packet (0000)
    // Then: the ref advertisement from git
    let mut body = Vec::new();
    let service_line = format!("# service={}\n", service);
    write_pkt_line(&mut body, service_line.as_bytes());
    body.extend_from_slice(b"0000");
    body.extend_from_slice(&advertisement);

    Ok(body)
}

/// The ref advertisement `service` (`git-upload-pack` or `git-receive-pack`)
/// opens a protocol v0 session with, as git writes it.
pub async fn ref_advertisement(repo_path: &Path, service: &str) -> Result<Vec<u8>> {
    validate_service(service)?;

    let output = Command::new("git")
        .arg(service.strip_prefix("git-").unwrap_or(service))
        .arg("--stateless-rpc")
        .arg("--advertise-refs")
        .arg(repo_path)
        .env_remove("GIT_PROTOCOL")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| DeltaError::Storage(format!("failed to run git: {}", e)))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::error!("git {} --advertise-refs failed: {}", service, stderr);
        return Err(DeltaError::Storage(format!(
            "git {} --advertise-refs failed",
            service
        )));
    }
    Ok(output.stdout)
}

/// Run `git-upload-pack --stateless-rpc` for clone/fetch, buffering I/O.
pub async fn upload_pack(repo_path: &Path, input: &[u8]) -> Result<Vec<u8>> {
    run_service_rpc(repo_path, "upload-pack", input).await
}

/// Spawn `git [-c key=value]... <service> --stateless-rpc <repo>` with piped
/// stdin and stdout, for callers that stream request and response bodies.
///
/// The child is killed if dropped. Its stderr is drained in the background
/// (and logged), so git can never block on a full stderr pipe.
pub fn spawn_service_rpc(
    repo_path: &Path,
    service: &str,
    config: &[(&str, &str)],
) -> Result<Child> {
    spawn_service(repo_path, service, config, true, None)
}

/// Spawn `git <service> <repo>` for a full-duplex session (the SSH
/// transport): the ref advertisement, negotiation and pack all flow over
/// one pair of pipes. `git_protocol` is the client's `GIT_PROTOCOL` request
/// (e.g. `version=2`).
///
/// The child is killed if dropped, and its stderr is drained into the log.
pub fn spawn_service_session(
    repo_path: &Path,
    service: &str,
    git_protocol: Option<&str>,
) -> Result<Child> {
    spawn_service(repo_path, service, &[], false, git_protocol)
}

fn spawn_service(
    repo_path: &Path,
    service: &str,
    config: &[(&str, &str)],
    stateless: bool,
    git_protocol: Option<&str>,
) -> Result<Child> {
    let service = service.strip_prefix("git-").unwrap_or(service);
    validate_service(&format!("git-{service}"))?;

    let mut command = Command::new("git");
    for (key, value) in config {
        command.arg("-c").arg(format!("{key}={value}"));
    }
    command.arg(service);
    if stateless {
        command.arg("--stateless-rpc");
    }
    match git_protocol {
        Some(protocol) => command.env("GIT_PROTOCOL", protocol),
        None => command.env_remove("GIT_PROTOCOL"),
    };
    let mut child = command
        .arg(repo_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| DeltaError::Storage(format!("failed to spawn git {}: {}", service, e)))?;

    if let Some(stderr) = child.stderr.take() {
        let service = service.to_string();
        tokio::spawn(async move {
            use tokio::io::AsyncReadExt;
            let mut buf = Vec::new();
            // Keep at most 64 KiB for the log; discard the rest.
            let _ = stderr.take(64 * 1024).read_to_end(&mut buf).await;
            if !buf.is_empty() {
                tracing::debug!(service, stderr = %String::from_utf8_lossy(&buf), "git stderr");
            }
        });
    }
    Ok(child)
}

/// Run a git service in stateless RPC mode with a buffered request.
async fn run_service_rpc(repo_path: &Path, service: &str, input: &[u8]) -> Result<Vec<u8>> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let mut child = spawn_service_rpc(repo_path, service, &[])?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| DeltaError::Storage("git stdin unavailable".into()))?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| DeltaError::Storage("git stdout unavailable".into()))?;

    // Write the request while reading the response: git may produce more
    // output than a pipe buffer holds before it has consumed all input.
    let write = async {
        let result = stdin.write_all(input).await;
        drop(stdin);
        result
    };
    let mut output = Vec::new();
    let (write_result, read_result) = tokio::join!(write, stdout.read_to_end(&mut output));
    read_result.map_err(|e| DeltaError::Storage(format!("git {} failed: {}", service, e)))?;
    if let Err(e) = write_result {
        // git exits early on some invalid requests; its output explains why.
        tracing::debug!(service, "git stopped reading its input: {}", e);
    }

    let status = child
        .wait()
        .await
        .map_err(|e| DeltaError::Storage(format!("git {} failed: {}", service, e)))?;
    if !status.success() {
        // Don't fail — git sometimes returns non-zero for valid operations
        // (e.g., rejected pushes). The client reads the error from stdout.
        tracing::warn!(service, %status, "git service exited with error");
    }

    Ok(output)
}

/// A single ref update requested by a push.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefUpdate {
    pub old: String,
    pub new: String,
    pub refname: String,
}

impl RefUpdate {
    /// The push creates the ref.
    pub fn is_create(&self) -> bool {
        is_null_oid(&self.old)
    }

    /// The push deletes the ref.
    pub fn is_delete(&self) -> bool {
        is_null_oid(&self.new)
    }
}

/// Whether `oid` is the all-zero object id git uses for "no object".
pub fn is_null_oid(oid: &str) -> bool {
    !oid.is_empty() && oid.bytes().all(|b| b == b'0')
}

/// Parsed command section of a `git-receive-pack` request.
#[derive(Debug, Default)]
pub struct ReceiveCommands {
    pub updates: Vec<RefUpdate>,
    pub capabilities: Vec<String>,
}

impl ReceiveCommands {
    pub fn has_capability(&self, name: &str) -> bool {
        self.capabilities.iter().any(|c| c == name)
    }
}

/// Parse the pkt-line command list that starts a receive-pack request
/// (`<old> <new> <ref>[\0<capabilities>]` lines up to a flush packet).
/// `shallow` lines are skipped. `section` must end with the flush packet.
pub fn parse_receive_commands(section: &[u8]) -> Result<ReceiveCommands> {
    let mut commands = ReceiveCommands::default();
    let mut rest = section;
    loop {
        let (payload, remaining) = split_pkt_line(rest)?;
        rest = remaining;
        let Some(payload) = payload else {
            return Ok(commands); // flush
        };
        let line = payload.strip_suffix(b"\n").unwrap_or(payload);
        let (line, caps) = match line.iter().position(|&b| b == 0) {
            Some(nul) => (&line[..nul], Some(&line[nul + 1..])),
            None => (line, None),
        };
        if let Some(caps) = caps
            && commands.capabilities.is_empty()
        {
            commands.capabilities = String::from_utf8_lossy(caps)
                .split_whitespace()
                .map(str::to_string)
                .collect();
        }
        let line = std::str::from_utf8(line)
            .map_err(|_| DeltaError::InvalidRef("non-UTF-8 push command".into()))?;
        if line.starts_with("shallow ") {
            continue;
        }
        let mut parts = line.splitn(3, ' ');
        match (parts.next(), parts.next(), parts.next()) {
            (Some(old), Some(new), Some(refname))
                if is_hex_oid(old) && is_hex_oid(new) && !refname.is_empty() =>
            {
                commands.updates.push(RefUpdate {
                    old: old.to_string(),
                    new: new.to_string(),
                    refname: refname.to_string(),
                });
            }
            _ => {
                return Err(DeltaError::InvalidRef(format!(
                    "malformed push command: {line}"
                )));
            }
        }
    }
}

/// Length of the command section at the start of `buf` (through the first
/// flush packet), or `None` if more data is needed.
pub fn receive_commands_len(buf: &[u8]) -> Result<Option<usize>> {
    let mut offset = 0;
    loop {
        let rest = &buf[offset..];
        if rest.len() < 4 {
            return Ok(None);
        }
        let len = pkt_len(&rest[..4])?;
        if len == 0 {
            return Ok(Some(offset + 4));
        }
        if rest.len() < len {
            return Ok(None);
        }
        offset += len;
    }
}

/// Build a receive-pack response that rejects every update with `reason`,
/// in the report-status format git clients display as
/// `! [remote rejected] <ref> (<reason>)`.
pub fn rejection_report(commands: &ReceiveCommands, reason: &str) -> Vec<u8> {
    let mut report = Vec::new();
    write_pkt_line(&mut report, b"unpack ok\n");
    for update in &commands.updates {
        let line = format!("ng {} {}\n", update.refname, reason.replace('\n', " "));
        write_pkt_line(&mut report, line.as_bytes());
    }
    report.extend_from_slice(b"0000");

    if !(commands.has_capability("side-band-64k") || commands.has_capability("side-band")) {
        return report;
    }
    // With side-band, the report travels on band 1 inside pkt-lines.
    let max_payload = if commands.has_capability("side-band-64k") {
        65515
    } else {
        995
    };
    let mut out = Vec::new();
    for chunk in report.chunks(max_payload - 1) {
        let mut packet = Vec::with_capacity(chunk.len() + 1);
        packet.push(1u8);
        packet.extend_from_slice(chunk);
        write_pkt_line(&mut out, &packet);
    }
    out.extend_from_slice(b"0000");
    out
}

fn is_hex_oid(s: &str) -> bool {
    matches!(s.len(), 40 | 64) && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Parse a 4-byte pkt-line length prefix.
fn pkt_len(prefix: &[u8]) -> Result<usize> {
    let hex = std::str::from_utf8(prefix)
        .ok()
        .and_then(|s| usize::from_str_radix(s, 16).ok())
        .ok_or_else(|| DeltaError::InvalidRef("invalid pkt-line length".into()))?;
    match hex {
        0 => Ok(0),
        1..=3 => Err(DeltaError::InvalidRef("invalid pkt-line length".into())),
        n => Ok(n),
    }
}

/// Split one pkt-line off `buf`: `(Some(payload), rest)`, or `(None, rest)`
/// for a flush packet.
fn split_pkt_line(buf: &[u8]) -> Result<(Option<&[u8]>, &[u8])> {
    if buf.len() < 4 {
        return Err(DeltaError::InvalidRef("truncated pkt-line".into()));
    }
    let len = pkt_len(&buf[..4])?;
    if len == 0 {
        return Ok((None, &buf[4..]));
    }
    if buf.len() < len {
        return Err(DeltaError::InvalidRef("truncated pkt-line".into()));
    }
    Ok((Some(&buf[4..len]), &buf[len..]))
}

/// Validate that the service name is one of the allowed git services.
fn validate_service(service: &str) -> Result<()> {
    match service {
        "git-upload-pack" | "git-receive-pack" => Ok(()),
        _ => Err(DeltaError::InvalidRef(format!(
            "invalid git service: {}",
            service
        ))),
    }
}

/// Write a pkt-line formatted message.
fn write_pkt_line(buf: &mut Vec<u8>, data: &[u8]) {
    let len = data.len() + 4; // 4 bytes for the length prefix itself
    buf.extend_from_slice(format!("{:04x}", len).as_bytes());
    buf.extend_from_slice(data);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_service() {
        assert!(validate_service("git-upload-pack").is_ok());
        assert!(validate_service("git-receive-pack").is_ok());
        assert!(validate_service("git-evil-command").is_err());
        assert!(validate_service("rm -rf").is_err());
    }

    const OLD: &str = "1111111111111111111111111111111111111111";
    const NEW: &str = "2222222222222222222222222222222222222222";
    const ZERO: &str = "0000000000000000000000000000000000000000";

    fn pkt(data: &str) -> Vec<u8> {
        let mut buf = Vec::new();
        write_pkt_line(&mut buf, data.as_bytes());
        buf
    }

    #[test]
    fn test_parse_receive_commands() {
        let mut section = pkt(&format!(
            "{OLD} {NEW} refs/heads/main\0report-status side-band-64k agent=git/2\n"
        ));
        section.extend(pkt(&format!("{NEW} {ZERO} refs/heads/old\n")));
        section.extend_from_slice(b"0000");
        let mut request = section.clone();
        request.extend_from_slice(b"PACK....");

        assert_eq!(receive_commands_len(&request).unwrap(), Some(section.len()));
        assert_eq!(receive_commands_len(&request[..10]).unwrap(), None);

        let cmds = parse_receive_commands(&section).unwrap();
        assert_eq!(cmds.updates.len(), 2);
        assert_eq!(cmds.updates[0].refname, "refs/heads/main");
        assert!(!cmds.updates[0].is_delete());
        assert!(cmds.updates[1].is_delete());
        assert!(cmds.has_capability("side-band-64k"));
        assert!(cmds.has_capability("report-status"));
    }

    #[test]
    fn test_parse_receive_commands_rejects_garbage() {
        let mut section = pkt("not a command\n");
        section.extend_from_slice(b"0000");
        assert!(parse_receive_commands(&section).is_err());
        assert!(receive_commands_len(b"zzzz").is_err());
        assert!(receive_commands_len(b"0002").is_err());
    }

    #[test]
    fn test_rejection_report_plain_and_sideband() {
        let mut section = pkt(&format!("{OLD} {NEW} refs/heads/main\0report-status\n"));
        section.extend_from_slice(b"0000");
        let plain = parse_receive_commands(&section).unwrap();
        let report = String::from_utf8(rejection_report(&plain, "protected branch")).unwrap();
        assert_eq!(
            report,
            "000eunpack ok\n0028ng refs/heads/main protected branch\n0000"
        );

        let mut section = pkt(&format!(
            "{OLD} {NEW} refs/heads/main\0report-status side-band-64k\n"
        ));
        section.extend_from_slice(b"0000");
        let banded = parse_receive_commands(&section).unwrap();
        let report = rejection_report(&banded, "protected branch");
        // One band-1 packet wrapping the plain report, then flush.
        assert_eq!(&report[4..5], &[1u8]);
        assert!(report.ends_with(b"0000"));
        assert_eq!(pkt_len(&report[..4]).unwrap(), report.len() - 4);
    }

    #[tokio::test]
    async fn test_service_rpc_does_not_deadlock_on_large_output() {
        // An invalid request makes git exit quickly; the point is that the
        // buffered helper returns instead of hanging.
        let tmp = tempfile::tempdir().unwrap();
        let status = std::process::Command::new("git")
            .args(["init", "--bare", "-q"])
            .arg(tmp.path())
            .status()
            .unwrap();
        assert!(status.success());
        let input = vec![b'0'; 1 << 20];
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(20),
            upload_pack(tmp.path(), &input),
        )
        .await;
        assert!(result.is_ok(), "upload_pack hung");
    }

    #[test]
    fn test_write_pkt_line() {
        let mut buf = Vec::new();
        write_pkt_line(&mut buf, b"# service=git-upload-pack\n");
        let s = String::from_utf8(buf).unwrap();
        // "# service=git-upload-pack\n" is 26 bytes + 4 = 30 = 0x001e
        assert!(s.starts_with("001e"));
        assert!(s.contains("# service=git-upload-pack"));
    }
}
