//! Git smart HTTP protocol handlers.
//!
//! Implements the server side of the git smart HTTP transport:
//! - `GET /info/refs?service=git-upload-pack` — ref advertisement for clone/fetch
//! - `GET /info/refs?service=git-receive-pack` — ref advertisement for push
//! - `POST /git-upload-pack` — pack negotiation and data transfer (clone/fetch)
//! - `POST /git-receive-pack` — receive pushed data

use std::path::Path;
use std::process::Stdio;
use tokio::process::Command;

use delta_core::{DeltaError, Result};

/// Run `git-upload-pack --advertise-refs` or `git-receive-pack --advertise-refs`
/// for the info/refs endpoint.
pub async fn advertise_refs(repo_path: &Path, service: &str) -> Result<Vec<u8>> {
    validate_service(service)?;

    let output = Command::new("git")
        .arg(service.strip_prefix("git-").unwrap_or(service))
        .arg("--stateless-rpc")
        .arg("--advertise-refs")
        .arg(repo_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| DeltaError::Storage(format!("failed to run git: {}", e)))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(DeltaError::Storage(format!(
            "git {} --advertise-refs failed: {}",
            service, stderr
        )));
    }

    // Build the smart HTTP response:
    // First line: pkt-line with "# service=git-upload-pack\n"
    // Then: flush packet (0000)
    // Then: the ref advertisement from git
    let mut body = Vec::new();
    let service_line = format!("# service={}\n", service);
    write_pkt_line(&mut body, service_line.as_bytes());
    body.extend_from_slice(b"0000");
    body.extend_from_slice(&output.stdout);

    Ok(body)
}

/// Run `git-upload-pack --stateless-rpc` for clone/fetch.
pub async fn upload_pack(repo_path: &Path, input: &[u8]) -> Result<Vec<u8>> {
    run_service_rpc(repo_path, "upload-pack", input).await
}

/// Run `git-receive-pack --stateless-rpc` for push.
pub async fn receive_pack(repo_path: &Path, input: &[u8]) -> Result<Vec<u8>> {
    run_service_rpc(repo_path, "receive-pack", input).await
}

/// Run a git service in stateless RPC mode.
async fn run_service_rpc(repo_path: &Path, service: &str, input: &[u8]) -> Result<Vec<u8>> {
    let mut child = Command::new("git")
        .arg(service)
        .arg("--stateless-rpc")
        .arg(repo_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| DeltaError::Storage(format!("failed to spawn git {}: {}", service, e)))?;

    // Write input to stdin
    if let Some(mut stdin) = child.stdin.take() {
        use tokio::io::AsyncWriteExt;
        stdin
            .write_all(input)
            .await
            .map_err(|e| DeltaError::Storage(format!("failed to write to git stdin: {}", e)))?;
        drop(stdin);
    }

    let output = child
        .wait_with_output()
        .await
        .map_err(|e| DeltaError::Storage(format!("git {} failed: {}", service, e)))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::warn!(service, %stderr, "git service exited with error");
        // Don't fail — git sometimes returns non-zero for valid operations
        // (e.g., rejected pushes). The client reads the error from stdout.
    }

    Ok(output.stdout)
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
