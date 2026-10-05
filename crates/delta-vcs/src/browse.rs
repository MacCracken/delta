//! Git repository browsing: tree listing, blob reading, log, blame, and commit details.

use delta_core::{DeltaError, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::Stdio;
use tokio::process::{Child, Command};

use crate::process::output_capped;
use crate::validate::validate_ref;

/// Most output kept from a log or listing command; entries past it are
/// dropped.
const MAX_LISTING_BYTES: usize = 32 * 1024 * 1024;
/// Most diff text produced for one commit or comparison.
pub const MAX_DIFF_BYTES: usize = 10 * 1024 * 1024;

/// A single entry in a git tree listing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TreeEntry {
    pub mode: String,
    pub kind: String,
    pub hash: String,
    pub name: String,
    pub path: String,
}

/// Commit log entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub sha: String,
    pub author_name: String,
    pub author_email: String,
    pub message: String,
    pub body: String,
    pub date: String,
}

/// A blame entry mapping a single line to a commit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlameLine {
    pub sha: String,
    pub author: String,
    pub date: String,
    pub line_number: usize,
    pub content: String,
}

/// Full commit detail including diff and stats.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitDetail {
    pub sha: String,
    pub author_name: String,
    pub author_email: String,
    pub author_date: String,
    pub committer_name: String,
    pub committer_email: String,
    pub committer_date: String,
    pub parents: Vec<String>,
    pub message: String,
    pub body: String,
    pub diff: String,
    /// The diff was too large to include.
    pub diff_truncated: bool,
    pub stats: Vec<CommitFileStat>,
}

/// File-level change statistics within a commit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitFileStat {
    pub path: String,
    pub additions: i64,
    pub deletions: i64,
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

/// Validate a repository-relative path for safety.
///
/// Rejects path traversal (`..`), absolute paths (leading `/`), and null bytes.
/// An empty string is valid and means the repository root.
fn validate_path(path: &str) -> Result<()> {
    if path.contains('\0') {
        return Err(DeltaError::InvalidRef("path contains null bytes".into()));
    }
    if path.starts_with('/') {
        return Err(DeltaError::InvalidRef(
            "path must not start with '/'".into(),
        ));
    }
    for component in path.split('/') {
        if component == ".." {
            return Err(DeltaError::InvalidRef(
                "path must not contain '..' components".into(),
            ));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tree listing
// ---------------------------------------------------------------------------

/// List entries in a git tree at the given revision and path.
///
/// Returns tree entries sorted with directories first, then files,
/// alphabetically within each group. `path` names a directory ("" for the
/// root); entry paths are relative to the repository root.
pub async fn list_tree(repo_path: &Path, rev: &str, path: &str) -> Result<Vec<TreeEntry>> {
    validate_ref(rev)?;
    validate_path(path)?;

    let mut args = vec!["ls-tree".to_string(), "-z".to_string(), rev.to_string()];
    let dir = path.trim_end_matches('/');
    if !dir.is_empty() {
        // The trailing slash lists the directory's children rather than
        // the directory entry itself.
        args.push("--".to_string());
        args.push(format!("{dir}/"));
    }

    let output = Command::new("git")
        .args(&args)
        .current_dir(repo_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| DeltaError::Storage(format!("failed to run git ls-tree: {}", e)))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::error!("git ls-tree failed: {}", stderr);
        return Err(DeltaError::Storage("git ls-tree failed".into()));
    }

    let mut entries: Vec<TreeEntry> = output
        .stdout
        .split(|&b| b == 0)
        .filter_map(parse_ls_tree_record)
        .map(|(mode, kind, hash, _size, entry_path)| TreeEntry {
            name: entry_path
                .rsplit('/')
                .next()
                .unwrap_or(&entry_path)
                .to_string(),
            mode,
            kind,
            hash,
            path: entry_path,
        })
        .collect();

    // Sort: trees (directories) first, then blobs, alphabetical within groups.
    entries.sort_by(|a, b| {
        let a_is_tree = a.kind == "tree";
        let b_is_tree = b.kind == "tree";
        b_is_tree
            .cmp(&a_is_tree)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    Ok(entries)
}

/// A file in a recursive tree listing.
#[derive(Debug, Clone)]
pub struct BlobEntry {
    pub path: String,
    pub size: u64,
}

/// List every file (blob) reachable from `rev`, with its size, using a
/// single `git ls-tree -r` (no per-directory recursion).
pub async fn list_blobs(repo_path: &Path, rev: &str) -> Result<Vec<BlobEntry>> {
    validate_ref(rev)?;
    let output = output_capped(
        Command::new("git")
            .args(["ls-tree", "-r", "-z", "-l", rev])
            .current_dir(repo_path),
        MAX_LISTING_BYTES,
    )
    .await?;
    if !output.status.success() && !output.truncated {
        tracing::error!("git ls-tree -r failed: {}", output.stderr);
        return Err(DeltaError::Storage("git ls-tree failed".into()));
    }
    Ok(complete_records(&output.stdout)
        .split(|&b| b == 0)
        .filter_map(parse_ls_tree_record)
        .filter(|(_, kind, _, _, _)| kind == "blob")
        .map(|(_, _, _, size, path)| BlobEntry {
            path,
            size: size.unwrap_or(0),
        })
        .collect())
}

/// The NUL-terminated records in `output`, without a cut-off last one.
fn complete_records(output: &[u8]) -> &[u8] {
    match output.iter().rposition(|&b| b == 0) {
        Some(end) => &output[..end],
        None => &[],
    }
}

/// Parse one NUL-terminated `git ls-tree -z [-l]` record:
/// `<mode> <type> <hash>[ <size>]\t<path>`.
fn parse_ls_tree_record(record: &[u8]) -> Option<(String, String, String, Option<u64>, String)> {
    let tab = record.iter().position(|&b| b == b'\t')?;
    let meta = std::str::from_utf8(&record[..tab]).ok()?;
    let path = String::from_utf8_lossy(&record[tab + 1..]).into_owned();
    let mut parts = meta.split_whitespace();
    let mode = parts.next()?.to_string();
    let kind = parts.next()?.to_string();
    let hash = parts.next()?.to_string();
    let size = parts.next().and_then(|s| s.parse().ok());
    Some((mode, kind, hash, size, path))
}

// ---------------------------------------------------------------------------
// Blob reading
// ---------------------------------------------------------------------------

/// The `<rev>:<path>` object name of a file, after validating both parts.
fn blob_object(rev: &str, path: &str) -> Result<String> {
    validate_ref(rev)?;
    validate_path(path)?;
    if path.is_empty() {
        return Err(DeltaError::InvalidRef(
            "path must not be empty for blob read".into(),
        ));
    }
    Ok(format!("{}:{}", rev, path))
}

/// Size in bytes of the file at the given revision and path, without
/// reading it.
pub async fn blob_size(repo_path: &Path, rev: &str, path: &str) -> Result<u64> {
    let object = blob_object(rev, path)?;
    let output = Command::new("git")
        .args(["cat-file", "-t", &object])
        .current_dir(repo_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .await
        .map_err(|e| DeltaError::Storage(format!("failed to run git cat-file: {}", e)))?;
    match std::str::from_utf8(&output.stdout).map(str::trim) {
        Ok("blob") if output.status.success() => {}
        _ if !output.status.success() => {
            return Err(DeltaError::NotFound(format!("{path} not found at {rev}")));
        }
        _ => return Err(DeltaError::InvalidRef(format!("{path} is not a file"))),
    }

    let output = Command::new("git")
        .args(["cat-file", "-s", &object])
        .current_dir(repo_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .await
        .map_err(|e| DeltaError::Storage(format!("failed to run git cat-file: {}", e)))?;
    std::str::from_utf8(&output.stdout)
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .filter(|_| output.status.success())
        .ok_or_else(|| DeltaError::Storage("git cat-file -s failed".into()))
}

/// Start `git cat-file blob <rev>:<path>`, whose stdout streams the file.
/// The child is killed if dropped; the caller must reap it.
pub fn spawn_blob_reader(repo_path: &Path, rev: &str, path: &str) -> Result<Child> {
    let object = blob_object(rev, path)?;
    Command::new("git")
        .args(["cat-file", "blob", &object])
        .current_dir(repo_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| DeltaError::Storage(format!("failed to run git cat-file: {}", e)))
}

/// Read the raw bytes of a file at the given revision and path, refusing
/// (with [`DeltaError::TooLarge`]) files larger than `max_bytes`. At most
/// `max_bytes + 1` bytes are ever held in memory.
pub async fn read_blob(repo_path: &Path, rev: &str, path: &str, max_bytes: u64) -> Result<Vec<u8>> {
    let object = blob_object(rev, path)?;
    let max_bytes = usize::try_from(max_bytes).unwrap_or(usize::MAX - 1);
    let output = output_capped(
        Command::new("git")
            .args(["cat-file", "blob", &object])
            .current_dir(repo_path),
        max_bytes,
    )
    .await?;
    if output.truncated {
        return Err(DeltaError::TooLarge(format!(
            "{path} is larger than {max_bytes} bytes"
        )));
    }
    if !output.status.success() {
        return Err(DeltaError::NotFound(format!("{path} not found at {rev}")));
    }
    Ok(output.stdout)
}

/// Read a file as text at the given revision and path; see [`read_blob`].
///
/// If the content is not valid UTF-8, lossy conversion is used.
pub async fn read_blob_text(
    repo_path: &Path,
    rev: &str,
    path: &str,
    max_bytes: u64,
) -> Result<String> {
    let bytes = read_blob(repo_path, rev, path, max_bytes).await?;
    match String::from_utf8(bytes) {
        Ok(s) => Ok(s),
        Err(e) => Ok(String::from_utf8_lossy(e.as_bytes()).into_owned()),
    }
}

// ---------------------------------------------------------------------------
// Log
// ---------------------------------------------------------------------------

/// Retrieve commit log for a given revision, optionally scoped to a file path.
pub async fn log(
    repo_path: &Path,
    rev: &str,
    path: Option<&str>,
    limit: usize,
) -> Result<Vec<LogEntry>> {
    validate_ref(rev)?;
    if let Some(p) = path {
        validate_path(p)?;
    }

    // NUL-terminated fields: git never emits NUL inside one (it cuts
    // message and identity text at NUL), so commit messages can't forge
    // entries the way they could with a text delimiter.
    let mut args = vec![
        "log".to_string(),
        "-z".to_string(),
        "--format=%H%x00%an%x00%ae%x00%aI%x00%s%x00%b".to_string(),
        format!("-n{}", limit),
        rev.to_string(),
    ];

    if let Some(p) = path
        && !p.is_empty()
    {
        args.push("--".to_string());
        args.push(p.to_string());
    }

    // Huge commit messages can't exhaust memory: entries past the cap are
    // dropped (a cut-off record is never parsed).
    let output = output_capped(
        Command::new("git").args(&args).current_dir(repo_path),
        MAX_LISTING_BYTES,
    )
    .await?;
    if !output.status.success() && !output.truncated {
        tracing::error!("git log failed: {}", output.stderr);
        return Err(DeltaError::Storage("git log failed".into()));
    }

    Ok(parse_log_records(&output.stdout))
}

/// Parse `git log -z` output in the six-field format [`log`] requests.
fn parse_log_records(output: &[u8]) -> Vec<LogEntry> {
    let fields: Vec<String> = complete_records(output)
        .split(|&b| b == 0)
        .map(|f| String::from_utf8_lossy(f).into_owned())
        .collect();
    let mut entries = Vec::new();
    for [sha, author_name, author_email, date, message, body] in fields.as_chunks::<6>().0 {
        if !matches!(sha.len(), 40 | 64) || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
            tracing::error!("unexpected git log output");
            break;
        }
        entries.push(LogEntry {
            sha: sha.clone(),
            author_name: author_name.clone(),
            author_email: author_email.clone(),
            date: date.clone(),
            message: message.clone(),
            body: body.trim().to_string(),
        });
    }
    entries
}

// ---------------------------------------------------------------------------
// Blame
// ---------------------------------------------------------------------------

/// Run git blame in porcelain mode and return per-line blame information.
/// Files larger than `max_bytes` are refused with [`DeltaError::TooLarge`].
pub async fn blame(
    repo_path: &Path,
    rev: &str,
    path: &str,
    max_bytes: u64,
) -> Result<Vec<BlameLine>> {
    let size = blob_size(repo_path, rev, path).await?;
    if size > max_bytes {
        return Err(DeltaError::TooLarge(format!(
            "{path} is larger than {max_bytes} bytes"
        )));
    }

    let output = Command::new("git")
        .args(["blame", "--porcelain", rev, "--", path])
        .current_dir(repo_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| DeltaError::Storage(format!("failed to run git blame: {}", e)))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::error!("git blame failed: {}", stderr);
        return Err(DeltaError::Storage("git blame failed".into()));
    }

    Ok(parse_blame_porcelain(&String::from_utf8_lossy(
        &output.stdout,
    )))
}

/// Parse `git blame --porcelain` output.
///
/// Each line is a header (`<sha> <orig_line> <final_line> [<group_size>]`),
/// commit details, then the tab-prefixed content. A commit's details are
/// only printed with the first line it is blamed for, so they are kept per
/// commit for its later lines.
fn parse_blame_porcelain(output: &str) -> Vec<BlameLine> {
    let mut results = Vec::new();
    // sha -> (author, date)
    let mut commits: std::collections::HashMap<String, (String, String)> =
        std::collections::HashMap::new();
    let mut current_sha = String::new();
    let mut current_line_number: usize = 0;

    for line in output.lines() {
        if let Some(content) = line.strip_prefix('\t') {
            // Content line — this terminates the current entry.
            let (author, date) = commits.get(&current_sha).cloned().unwrap_or_default();
            results.push(BlameLine {
                sha: current_sha.clone(),
                author,
                date,
                line_number: current_line_number,
                content: content.to_string(),
            });
        } else if let Some(rest) = line.strip_prefix("author ") {
            commits.entry(current_sha.clone()).or_default().0 = rest.to_string();
        } else if let Some(rest) = line.strip_prefix("author-time ") {
            // Convert epoch timestamp to ISO 8601 (UTC).
            if let Ok(epoch) = rest.trim().parse::<i64>() {
                commits.entry(current_sha.clone()).or_default().1 = epoch_to_iso8601(epoch);
            }
        } else {
            let parts: Vec<&str> = line.split(' ').collect();
            let is_header = parts.len() >= 3
                && matches!(parts[0].len(), 40 | 64)
                && parts[0].bytes().all(|b| b.is_ascii_hexdigit());
            if is_header && let Ok(n) = parts[2].parse::<usize>() {
                current_sha = parts[0].to_string();
                current_line_number = n;
            }
        }
    }

    results
}

/// Convert a Unix epoch timestamp to an ISO 8601 UTC string.
fn epoch_to_iso8601(epoch: i64) -> String {
    // Manual conversion to avoid pulling in chrono just for this.
    // We produce a UTC date-time: "YYYY-MM-DDTHH:MM:SSZ".
    const SECS_PER_DAY: i64 = 86400;
    const SECS_PER_HOUR: i64 = 3600;
    const SECS_PER_MIN: i64 = 60;

    let mut days = epoch / SECS_PER_DAY;
    let mut day_secs = epoch % SECS_PER_DAY;
    if day_secs < 0 {
        days -= 1;
        day_secs += SECS_PER_DAY;
    }

    let hours = day_secs / SECS_PER_HOUR;
    let minutes = (day_secs % SECS_PER_HOUR) / SECS_PER_MIN;
    let seconds = day_secs % SECS_PER_MIN;

    // Days since Unix epoch (1970-01-01) to (year, month, day).
    // Algorithm from Howard Hinnant's civil_from_days.
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };

    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        y, m, d, hours, minutes, seconds
    )
}

// ---------------------------------------------------------------------------
// Show commit
// ---------------------------------------------------------------------------

/// Show one commit: metadata, per-file stats and its diff against its
/// first parent. Diffs over [`MAX_DIFF_BYTES`] are left out
/// (`diff_truncated`).
pub async fn show_commit(repo_path: &Path, sha: &str) -> Result<CommitDetail> {
    validate_ref(sha)?;

    // 1. Metadata, as NUL-terminated fields (see `log`).
    let meta = output_capped(
        Command::new("git")
            .args([
                "log",
                "-1",
                "-z",
                "--format=%H%x00%an%x00%ae%x00%aI%x00%cn%x00%ce%x00%cI%x00%P%x00%s%x00%b",
                sha,
            ])
            .current_dir(repo_path),
        MAX_LISTING_BYTES,
    )
    .await?;
    if !meta.status.success() && !meta.truncated {
        tracing::error!("git log -1 failed: {}", meta.stderr);
        return Err(DeltaError::Storage("git log failed".into()));
    }
    let fields: Vec<String> = meta
        .stdout
        .split(|&b| b == 0)
        .map(|f| String::from_utf8_lossy(f).into_owned())
        .collect();
    let commit_sha = fields.first().cloned().unwrap_or_default();
    if fields.len() < 10
        || !matches!(commit_sha.len(), 40 | 64)
        || !commit_sha.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(DeltaError::Storage(
            "unexpected git log output format".into(),
        ));
    }
    let parents: Vec<String> = fields[7].split_whitespace().map(str::to_string).collect();

    // Compare with the first parent; a root commit with nothing
    // (`diff-tree --root`).
    let diff_command = |options: &[&str]| {
        let mut command = Command::new("git");
        command
            .args(["-c", "core.quotePath=false"])
            .current_dir(repo_path);
        match parents.first() {
            Some(parent) => command
                .arg("diff")
                .args(options)
                .args([parent.as_str(), commit_sha.as_str()]),
            None => command
                .args(["diff-tree", "-r", "--root", "--no-commit-id"])
                .args(options)
                .arg(&commit_sha),
        };
        command
    };

    // 2. Per-file stats.
    let numstat = output_capped(&mut diff_command(&["--numstat", "-z"]), MAX_LISTING_BYTES).await?;
    let stats = crate::diff::parse_numstat_z(&numstat.stdout)
        .into_iter()
        .map(|(additions, deletions, path)| CommitFileStat {
            path,
            additions,
            deletions,
        })
        .collect();

    // 3. The unified diff.
    let diff_output = output_capped(&mut diff_command(&["-p"]), MAX_DIFF_BYTES).await?;
    let diff_truncated = diff_output.truncated;
    let diff = if diff_truncated {
        String::new()
    } else {
        String::from_utf8_lossy(&diff_output.stdout).into_owned()
    };

    Ok(CommitDetail {
        sha: commit_sha,
        author_name: fields[1].clone(),
        author_email: fields[2].clone(),
        author_date: fields[3].clone(),
        committer_name: fields[4].clone(),
        committer_email: fields[5].clone(),
        committer_date: fields[6].clone(),
        parents,
        message: fields[8].clone(),
        body: fields[9].trim().to_string(),
        diff,
        diff_truncated,
        stats,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_path_ok() {
        assert!(validate_path("").is_ok());
        assert!(validate_path("src/main.rs").is_ok());
        assert!(validate_path("a/b/c").is_ok());
        assert!(validate_path("file.txt").is_ok());
    }

    #[test]
    fn test_validate_path_rejects_traversal() {
        assert!(validate_path("..").is_err());
        assert!(validate_path("a/../b").is_err());
        assert!(validate_path("../etc/passwd").is_err());
    }

    #[test]
    fn test_validate_path_rejects_absolute() {
        assert!(validate_path("/etc/passwd").is_err());
    }

    #[test]
    fn test_validate_path_rejects_null() {
        assert!(validate_path("a\0b").is_err());
    }

    #[test]
    fn test_epoch_to_iso8601() {
        // 2024-01-01T00:00:00Z == 1704067200
        assert_eq!(epoch_to_iso8601(1_704_067_200), "2024-01-01T00:00:00Z");
        // Unix epoch
        assert_eq!(epoch_to_iso8601(0), "1970-01-01T00:00:00Z");
    }
}
