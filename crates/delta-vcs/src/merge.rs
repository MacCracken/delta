//! Git merge execution for pull requests.

use delta_core::{DeltaError, Result};
use std::path::Path;
use std::process::Stdio;
use tokio::process::Command;

use crate::validate::validate_ref;

/// Execute a merge in a bare repository using a temporary worktree.
/// Returns the resulting merge commit SHA.
pub async fn execute_merge(
    repo_path: &Path,
    base_branch: &str,
    head_branch: &str,
    strategy: MergeMode,
    merge_message: &str,
    author_name: &str,
    author_email: &str,
) -> Result<String> {
    validate_ref(base_branch)?;
    validate_ref(head_branch)?;

    let tmp_dir = tempfile::tempdir()
        .map_err(|e| DeltaError::Storage(format!("failed to create temp dir: {}", e)))?;
    let worktree_path = tmp_dir.path().join("merge-worktree");
    let worktree_str = worktree_path.to_str()
        .ok_or_else(|| DeltaError::Storage("worktree path is not valid UTF-8".into()))?;

    // Add worktree at the base branch
    run_git(repo_path, &["worktree", "add", worktree_str, base_branch]).await?;

    // Set author info
    run_git_in(&worktree_path, &["config", "user.name", author_name]).await?;
    run_git_in(&worktree_path, &["config", "user.email", author_email]).await?;

    let merge_result = do_merge(&worktree_path, head_branch, strategy, merge_message).await;

    if let Err(e) = merge_result {
        let _ = run_git(repo_path, &["worktree", "remove", "--force", worktree_str]).await;
        return Err(e);
    }

    // Get the resulting commit SHA
    let sha = run_git_output(&worktree_path, &["rev-parse", "HEAD"]).await?;

    // Clean up worktree
    let _ = run_git(repo_path, &["worktree", "remove", "--force", worktree_str]).await;

    Ok(sha.trim().to_string())
}

async fn do_merge(
    worktree: &Path,
    head_branch: &str,
    strategy: MergeMode,
    message: &str,
) -> Result<()> {
    match strategy {
        MergeMode::Merge => {
            // Try direct branch ref first
            run_git_in(worktree, &["merge", "--no-ff", "-m", message, head_branch]).await
        }
        MergeMode::Squash => {
            run_git_in(worktree, &["merge", "--squash", head_branch]).await?;
            run_git_in(worktree, &["commit", "-m", message]).await
        }
        MergeMode::Rebase => {
            run_git_in(worktree, &["rebase", head_branch]).await
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum MergeMode {
    Merge,
    Squash,
    Rebase,
}

async fn run_git(repo_path: &Path, args: &[&str]) -> Result<()> {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| DeltaError::Storage(format!("git failed: {}", e)))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(DeltaError::Storage(format!(
            "git {} failed: {}",
            args.first().unwrap_or(&""),
            stderr
        )));
    }
    Ok(())
}

async fn run_git_in(worktree: &Path, args: &[&str]) -> Result<()> {
    let output = Command::new("git")
        .args(args)
        .current_dir(worktree)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| DeltaError::Storage(format!("git failed: {}", e)))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(DeltaError::Storage(format!(
            "git {} failed: {}",
            args.first().unwrap_or(&""),
            stderr
        )));
    }
    Ok(())
}

async fn run_git_output(worktree: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(worktree)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| DeltaError::Storage(format!("git failed: {}", e)))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(DeltaError::Storage(format!(
            "git {} failed: {}",
            args.first().unwrap_or(&""),
            stderr
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}
