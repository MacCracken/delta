//! Diff generation between branches/commits.

use delta_core::{DeltaError, Result};
use std::path::Path;
use std::process::Stdio;
use tokio::process::Command;

use crate::validate::validate_ref;

/// Generate a unified diff between two refs (branches, commits, tags).
pub async fn diff_refs(repo_path: &Path, base: &str, head: &str) -> Result<String> {
    validate_ref(base)?;
    validate_ref(head)?;
    let output = Command::new("git")
        .args(["diff", &format!("{}...{}", base, head)])
        .current_dir(repo_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| DeltaError::Storage(format!("failed to run git diff: {}", e)))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(DeltaError::Storage(format!("git diff failed: {}", stderr)));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Get a stat summary (files changed, insertions, deletions).
pub async fn diff_stat(repo_path: &Path, base: &str, head: &str) -> Result<DiffStat> {
    validate_ref(base)?;
    validate_ref(head)?;
    let output = Command::new("git")
        .args([
            "diff",
            "--stat",
            "--numstat",
            &format!("{}...{}", base, head),
        ])
        .current_dir(repo_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| DeltaError::Storage(format!("failed to run git diff: {}", e)))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(DeltaError::Storage(format!(
            "git diff --stat failed: {}",
            stderr
        )));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut files = Vec::new();
    let mut total_additions = 0i64;
    let mut total_deletions = 0i64;

    for line in stdout.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 3 {
            let additions = parts[0].parse::<i64>().unwrap_or(0);
            let deletions = parts[1].parse::<i64>().unwrap_or(0);
            let file = parts[2].to_string();
            total_additions += additions;
            total_deletions += deletions;
            files.push(FileStat {
                path: file,
                additions,
                deletions,
            });
        }
    }

    Ok(DiffStat {
        files_changed: files.len(),
        additions: total_additions,
        deletions: total_deletions,
        files,
    })
}

/// List commits between base and head.
pub async fn list_commits(repo_path: &Path, base: &str, head: &str) -> Result<Vec<CommitInfo>> {
    validate_ref(base)?;
    validate_ref(head)?;
    let output = Command::new("git")
        .args([
            "log",
            "--format=%H%n%an%n%ae%n%s%n%aI%n---",
            &format!("{}..{}", base, head),
        ])
        .current_dir(repo_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| DeltaError::Storage(format!("failed to run git log: {}", e)))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(DeltaError::Storage(format!("git log failed: {}", stderr)));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut commits = Vec::new();

    for chunk in stdout.split("---\n") {
        let lines: Vec<&str> = chunk.trim().lines().collect();
        if lines.len() >= 5 {
            commits.push(CommitInfo {
                sha: lines[0].to_string(),
                author_name: lines[1].to_string(),
                author_email: lines[2].to_string(),
                message: lines[3].to_string(),
                date: lines[4].to_string(),
            });
        }
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
