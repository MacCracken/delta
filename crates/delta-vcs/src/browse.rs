//! Git repository browsing: tree listing, blob reading, log, blame, and commit details.

use delta_core::{DeltaError, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::process::Stdio;
use tokio::process::Command;

use crate::validate::validate_ref;

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
/// alphabetically within each group.
pub async fn list_tree(repo_path: &Path, rev: &str, path: &str) -> Result<Vec<TreeEntry>> {
    validate_ref(rev)?;
    validate_path(path)?;

    let mut args = vec!["ls-tree".to_string()];

    if path.is_empty() {
        args.push(rev.to_string());
    } else {
        args.push(rev.to_string());
        args.push("--".to_string());
        args.push(path.to_string());
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

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut entries = Vec::new();

    for line in stdout.lines() {
        if line.is_empty() {
            continue;
        }
        // Format: "mode type hash\tname"
        let Some((meta, name)) = line.split_once('\t') else {
            continue;
        };
        let parts: Vec<&str> = meta.split_whitespace().collect();
        if parts.len() < 3 {
            continue;
        }

        let entry_path = if path.is_empty() {
            name.to_string()
        } else {
            // When ls-tree is given a directory path it returns entries with
            // the full relative path already, so use it directly.
            // When given a file path the name is just the filename component.
            if name.contains('/') {
                name.to_string()
            } else {
                let trimmed = path.trim_end_matches('/');
                format!("{}/{}", trimmed, name)
            }
        };

        entries.push(TreeEntry {
            mode: parts[0].to_string(),
            kind: parts[1].to_string(),
            hash: parts[2].to_string(),
            name: name.rsplit('/').next().unwrap_or(name).to_string(),
            path: entry_path,
        });
    }

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

// ---------------------------------------------------------------------------
// Blob reading
// ---------------------------------------------------------------------------

/// Read the raw bytes of a file at the given revision and path.
pub async fn read_blob(repo_path: &Path, rev: &str, path: &str) -> Result<Vec<u8>> {
    validate_ref(rev)?;
    validate_path(path)?;

    if path.is_empty() {
        return Err(DeltaError::InvalidRef(
            "path must not be empty for blob read".into(),
        ));
    }

    let object = format!("{}:{}", rev, path);

    let output = Command::new("git")
        .args(["show", &object])
        .current_dir(repo_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| DeltaError::Storage(format!("failed to run git show: {}", e)))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::error!("git show failed: {}", stderr);
        return Err(DeltaError::Storage("git show failed".into()));
    }

    Ok(output.stdout)
}

/// Read a file as text at the given revision and path.
///
/// If the content is not valid UTF-8, lossy conversion is used.
pub async fn read_blob_text(repo_path: &Path, rev: &str, path: &str) -> Result<String> {
    let bytes = read_blob(repo_path, rev, path).await?;
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

    let mut args = vec![
        "log".to_string(),
        "--format=%H%n%an%n%ae%n%aI%n%s%n%b%n---END---".to_string(),
        format!("-n{}", limit),
        rev.to_string(),
    ];

    if let Some(p) = path
        && !p.is_empty()
    {
        args.push("--".to_string());
        args.push(p.to_string());
    }

    let output = Command::new("git")
        .args(&args)
        .current_dir(repo_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| DeltaError::Storage(format!("failed to run git log: {}", e)))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::error!("git log failed: {}", stderr);
        return Err(DeltaError::Storage("git log failed".into()));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut entries = Vec::new();

    for chunk in stdout.split("---END---\n") {
        let chunk = chunk.trim();
        if chunk.is_empty() {
            continue;
        }

        let lines: Vec<&str> = chunk.lines().collect();
        if lines.len() < 5 {
            continue;
        }

        let sha = lines[0].to_string();
        let author_name = lines[1].to_string();
        let author_email = lines[2].to_string();
        let date = lines[3].to_string();
        let message = lines[4].to_string();
        let body = if lines.len() > 5 {
            lines[5..].join("\n").trim().to_string()
        } else {
            String::new()
        };

        entries.push(LogEntry {
            sha,
            author_name,
            author_email,
            message,
            body,
            date,
        });
    }

    Ok(entries)
}

// ---------------------------------------------------------------------------
// Blame
// ---------------------------------------------------------------------------

/// Run git blame in porcelain mode and return per-line blame information.
pub async fn blame(repo_path: &Path, rev: &str, path: &str) -> Result<Vec<BlameLine>> {
    validate_ref(rev)?;
    validate_path(path)?;

    if path.is_empty() {
        return Err(DeltaError::InvalidRef(
            "path must not be empty for blame".into(),
        ));
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

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut results = Vec::new();

    // Porcelain format: blocks starting with `sha orig_line final_line [group]`,
    // then header lines, then a tab-prefixed content line.
    let mut current_sha = String::new();
    let mut current_author = String::new();
    let mut current_date = String::new();
    let mut current_line_number: usize = 0;

    for line in stdout.lines() {
        if let Some(content) = line.strip_prefix('\t') {
            // Content line — this terminates the current entry.
            results.push(BlameLine {
                sha: current_sha.clone(),
                author: current_author.clone(),
                date: current_date.clone(),
                line_number: current_line_number,
                content: content.to_string(),
            });
        } else if let Some(rest) = line.strip_prefix("author ") {
            current_author = rest.to_string();
        } else if let Some(rest) = line.strip_prefix("author-time ") {
            // Convert epoch timestamp to ISO 8601 (UTC).
            if let Ok(epoch) = rest.trim().parse::<i64>() {
                current_date = epoch_to_iso8601(epoch);
            }
        } else if !line.starts_with("author-")
            && !line.starts_with("committer")
            && !line.starts_with("summary ")
            && !line.starts_with("previous ")
            && !line.starts_with("filename ")
            && !line.starts_with("boundary")
        {
            // Possibly a header line: `sha orig_line final_line [group_size]`
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 3 {
                let maybe_sha = parts[0];
                // SHA is 40 hex chars (or 4+ for abbreviated, but porcelain gives full).
                if maybe_sha.len() >= 4 && maybe_sha.chars().all(|c| c.is_ascii_hexdigit()) {
                    current_sha = maybe_sha.to_string();
                    if let Ok(n) = parts[2].parse::<usize>() {
                        current_line_number = n;
                    }
                }
            }
        }
    }

    Ok(results)
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

/// Get full details of a single commit, including diff and file stats.
pub async fn show_commit(repo_path: &Path, sha: &str) -> Result<CommitDetail> {
    validate_ref(sha)?;

    // 1. Fetch metadata via git log -1.
    let output = Command::new("git")
        .args([
            "log",
            "-1",
            "--format=%H%n%an%n%ae%n%aI%n%cn%n%ce%n%cI%n%P%n%s%n%b%n---END---",
            sha,
        ])
        .current_dir(repo_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| DeltaError::Storage(format!("failed to run git log: {}", e)))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::error!("git log -1 failed: {}", stderr);
        return Err(DeltaError::Storage("git log failed".into()));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    if lines.len() < 9 {
        return Err(DeltaError::Storage(
            "unexpected git log output format".into(),
        ));
    }

    let commit_sha = lines[0].to_string();
    let author_name = lines[1].to_string();
    let author_email = lines[2].to_string();
    let author_date = lines[3].to_string();
    let committer_name = lines[4].to_string();
    let committer_email = lines[5].to_string();
    let committer_date = lines[6].to_string();
    let parents: Vec<String> = lines[7].split_whitespace().map(|s| s.to_string()).collect();
    let message = lines[8].to_string();

    // Body: everything between subject line and ---END--- marker.
    let body = {
        let after_subject = &lines[9..];
        let end_idx = after_subject
            .iter()
            .position(|l| *l == "---END---")
            .unwrap_or(after_subject.len());
        after_subject[..end_idx].join("\n").trim().to_string()
    };

    // 2. Fetch numstat for file-level stats.
    let stat_output = Command::new("git")
        .args([
            "diff",
            "--numstat",
            &format!("{}^..{}", commit_sha, commit_sha),
        ])
        .current_dir(repo_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await;

    let mut stats = Vec::new();
    // For root commits (no parent), diff sha^..sha fails. Fall back to
    // diff-tree against empty tree.
    let stat_stdout = match stat_output {
        Ok(ref o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
        _ => {
            // Root commit: diff against empty tree.
            let empty_tree = "4b825dc642cb6eb9a060e54bf899d15006c1b7a8";
            let fallback = Command::new("git")
                .args(["diff", "--numstat", empty_tree, &commit_sha])
                .current_dir(repo_path)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output()
                .await
                .map_err(|e| DeltaError::Storage(format!("failed to run git diff: {}", e)))?;
            String::from_utf8_lossy(&fallback.stdout).to_string()
        }
    };

    for line in stat_stdout.lines() {
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() >= 3 {
            stats.push(CommitFileStat {
                additions: parts[0].parse::<i64>().unwrap_or(0),
                deletions: parts[1].parse::<i64>().unwrap_or(0),
                path: parts[2].to_string(),
            });
        }
    }

    // 3. Fetch unified diff.
    let diff_output = Command::new("git")
        .args(["diff", &format!("{}^..{}", commit_sha, commit_sha)])
        .current_dir(repo_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await;

    let diff = match diff_output {
        Ok(ref o) if o.status.success() => String::from_utf8_lossy(&o.stdout).to_string(),
        _ => {
            // Root commit fallback.
            let empty_tree = "4b825dc642cb6eb9a060e54bf899d15006c1b7a8";
            let fallback = Command::new("git")
                .args(["diff", empty_tree, &commit_sha])
                .current_dir(repo_path)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output()
                .await
                .map_err(|e| DeltaError::Storage(format!("failed to run git diff: {}", e)))?;
            String::from_utf8_lossy(&fallback.stdout).to_string()
        }
    };

    Ok(CommitDetail {
        sha: commit_sha,
        author_name,
        author_email,
        author_date,
        committer_name,
        committer_email,
        committer_date,
        parents,
        message,
        body,
        diff,
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
