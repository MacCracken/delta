//! Diff generation between branches/commits.

use delta_core::{DeltaError, Result};
use std::path::Path;
use std::process::Stdio;
use tokio::process::Command;

use crate::browse::MAX_DIFF_BYTES;
use crate::process::output_capped;
use crate::validate::validate_ref;

/// Most output kept from a log or numstat listing.
const MAX_LISTING_BYTES: usize = 32 * 1024 * 1024;

/// Generate a unified diff between two refs (branches, commits, tags).
/// Diffs over [`MAX_DIFF_BYTES`] are refused with [`DeltaError::TooLarge`].
pub async fn diff_refs(repo_path: &Path, base: &str, head: &str) -> Result<String> {
    validate_ref(base)?;
    validate_ref(head)?;
    let output = output_capped(
        Command::new("git")
            .args([
                "-c",
                "core.quotePath=false",
                "diff",
                &format!("{}...{}", base, head),
            ])
            .current_dir(repo_path),
        MAX_DIFF_BYTES,
    )
    .await?;
    if output.truncated {
        return Err(DeltaError::TooLarge(format!(
            "diff is larger than {MAX_DIFF_BYTES} bytes"
        )));
    }
    if !output.status.success() {
        tracing::error!("git diff failed: {}", output.stderr);
        return Err(DeltaError::Storage("git diff failed".into()));
    }

    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Get a stat summary (files changed, insertions, deletions).
pub async fn diff_stat(repo_path: &Path, base: &str, head: &str) -> Result<DiffStat> {
    validate_ref(base)?;
    validate_ref(head)?;
    let output = output_capped(
        Command::new("git")
            .args(["diff", "--numstat", "-z", &format!("{}...{}", base, head)])
            .current_dir(repo_path),
        MAX_LISTING_BYTES,
    )
    .await?;
    if !output.status.success() && !output.truncated {
        tracing::error!("git diff --numstat failed: {}", output.stderr);
        return Err(DeltaError::Storage("git diff --numstat failed".into()));
    }

    let files: Vec<FileStat> = parse_numstat_z(&output.stdout)
        .into_iter()
        .map(|(additions, deletions, path)| FileStat {
            path,
            additions,
            deletions,
        })
        .collect();
    Ok(DiffStat {
        files_changed: files.len(),
        additions: files.iter().map(|f| f.additions).sum(),
        deletions: files.iter().map(|f| f.deletions).sum(),
        files,
    })
}

/// Parse `git diff --numstat -z` output into `(additions, deletions, path)`:
/// records are `<added>\t<deleted>\t<path>\0`, or for a rename
/// `<added>\t<deleted>\t\0<old path>\0<new path>\0`. Binary files (`-`)
/// count as zero lines; a record cut off by an output cap is dropped.
pub(crate) fn parse_numstat_z(output: &[u8]) -> Vec<(i64, i64, String)> {
    let complete = match output.iter().rposition(|&b| b == 0) {
        Some(end) => &output[..=end],
        None => return Vec::new(),
    };
    let mut fields = complete.split(|&b| b == 0);
    let mut stats = Vec::new();
    while let Some(record) = fields.next() {
        if record.is_empty() {
            continue;
        }
        let record = String::from_utf8_lossy(record);
        let mut parts = record.splitn(3, '\t');
        let (Some(additions), Some(deletions), Some(path)) =
            (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        let path = if path.is_empty() {
            // A rename: the old and new paths follow as separate fields.
            match (fields.next(), fields.next()) {
                (Some(_old), Some(new)) if !new.is_empty() => {
                    String::from_utf8_lossy(new).into_owned()
                }
                _ => break,
            }
        } else {
            path.to_string()
        };
        stats.push((
            additions.parse().unwrap_or(0),
            deletions.parse().unwrap_or(0),
            path,
        ));
    }
    stats
}

/// List commits between base and head.
pub async fn list_commits(repo_path: &Path, base: &str, head: &str) -> Result<Vec<CommitInfo>> {
    validate_ref(base)?;
    validate_ref(head)?;
    // NUL-terminated fields, as in `browse::log`: commit text can't forge
    // or corrupt entries.
    let output = output_capped(
        Command::new("git")
            .args([
                "log",
                "-z",
                "--format=%H%x00%an%x00%ae%x00%s%x00%aI",
                &format!("{}..{}", base, head),
            ])
            .current_dir(repo_path),
        MAX_LISTING_BYTES,
    )
    .await?;
    if !output.status.success() && !output.truncated {
        tracing::error!("git log failed: {}", output.stderr);
        return Err(DeltaError::Storage("git log failed".into()));
    }

    let complete = match output.stdout.iter().rposition(|&b| b == 0) {
        Some(end) => &output.stdout[..end],
        None => &[][..],
    };
    let fields: Vec<String> = complete
        .split(|&b| b == 0)
        .map(|f| String::from_utf8_lossy(f).into_owned())
        .collect();
    let mut commits = Vec::new();
    for [sha, author_name, author_email, message, date] in fields.as_chunks::<5>().0 {
        if !matches!(sha.len(), 40 | 64) || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
            tracing::error!("unexpected git log output");
            break;
        }
        commits.push(CommitInfo {
            sha: sha.clone(),
            author_name: author_name.clone(),
            author_email: author_email.clone(),
            message: message.clone(),
            date: date.clone(),
        });
    }

    Ok(commits)
}

/// Check if a merge would have conflicts.
pub async fn check_mergeable(repo_path: &Path, base: &str, head: &str) -> Result<bool> {
    validate_ref(base)?;
    validate_ref(head)?;
    let output = Command::new("git")
        .args(["merge-base", base, head])
        .current_dir(repo_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| DeltaError::Storage(format!("failed to check merge-base: {}", e)))?;

    // If merge-base succeeds, the branches share history and are potentially mergeable
    // A more thorough check would do a trial merge, but this is a reasonable first pass
    Ok(output.status.success())
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DiffStat {
    pub files_changed: usize,
    pub additions: i64,
    pub deletions: i64,
    pub files: Vec<FileStat>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FileStat {
    pub path: String,
    pub additions: i64,
    pub deletions: i64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CommitInfo {
    pub sha: String,
    pub author_name: String,
    pub author_email: String,
    pub message: String,
    pub date: String,
}
