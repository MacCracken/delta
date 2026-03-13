//! Git operations for remote agent workspaces.
//!
//! Uses temporary worktrees for atomic file writes, same pattern as merge.rs.

use delta_core::{DeltaError, Result};
use std::path::Path;
use std::process::Stdio;
use tokio::process::Command;

/// Maximum file size per file (10 MB).
const MAX_FILE_SIZE: usize = 10 * 1024 * 1024;

/// Maximum number of files per commit.
const MAX_FILES_PER_COMMIT: usize = 100;

/// A file operation in a workspace commit.
pub struct FileWrite {
    pub path: String,
    /// None means delete.
    pub content: Option<Vec<u8>>,
}

/// Create a workspace branch from a base ref.
pub async fn create_workspace_branch(
    repo_path: &Path,
    branch_name: &str,
    base_ref: &str,
) -> Result<String> {
    crate::validate::validate_ref(branch_name)?;
    crate::validate::validate_ref(base_ref)?;

    // Get the commit SHA for the base ref
    let base_sha = run_git_output(repo_path, &["rev-parse", base_ref]).await?;
    let base_sha = base_sha.trim();

    // Create the branch
    run_git(repo_path, &["branch", branch_name, base_sha]).await?;

    Ok(base_sha.to_string())
}

/// Commit files to a workspace branch using a temporary worktree.
/// Returns the new HEAD commit SHA.
pub async fn commit_workspace_files(
    repo_path: &Path,
    branch_name: &str,
    files: &[FileWrite],
    message: &str,
    author_name: &str,
    author_email: &str,
) -> Result<String> {
    crate::validate::validate_ref(branch_name)?;
    validate_author(author_name, author_email)?;

    if files.is_empty() {
        return Err(DeltaError::InvalidRef("no files to commit".into()));
    }
    if files.len() > MAX_FILES_PER_COMMIT {
        return Err(DeltaError::InvalidRef(format!(
            "too many files: {} (max {})",
            files.len(),
            MAX_FILES_PER_COMMIT
        )));
    }

    // Validate all paths and sizes
    for f in files {
        validate_file_path(&f.path)?;
        if let Some(ref content) = f.content
            && content.len() > MAX_FILE_SIZE
        {
            return Err(DeltaError::InvalidRef(format!(
                "file '{}' exceeds 10MB limit",
                f.path
            )));
        }
    }

    let tmp_dir = tempfile::tempdir()
        .map_err(|e| DeltaError::Storage(format!("failed to create temp dir: {}", e)))?;
    let worktree_path = tmp_dir.path().join("ws-worktree");
    let worktree_str = worktree_path
        .to_str()
        .ok_or_else(|| DeltaError::Storage("worktree path is not valid UTF-8".into()))?;

    // Add worktree at the workspace branch
    run_git(repo_path, &["worktree", "add", worktree_str, branch_name]).await?;

    // Set author info
    run_git_in(&worktree_path, &["config", "user.name", author_name]).await?;
    run_git_in(&worktree_path, &["config", "user.email", author_email]).await?;

    // Write/delete files
    let result = write_and_commit(&worktree_path, files, message).await;

    // Get HEAD SHA before cleanup (only if commit succeeded)
    let sha = match result {
        Ok(()) => run_git_output(&worktree_path, &["rev-parse", "HEAD"]).await,
        Err(e) => {
            let _ = run_git(repo_path, &["worktree", "remove", "--force", worktree_str]).await;
            return Err(e);
        }
    };

    // Clean up worktree
    let _ = run_git(repo_path, &["worktree", "remove", "--force", worktree_str]).await;

    let sha = sha?;
    Ok(sha.trim().to_string())
}

/// Delete a workspace branch.
pub async fn delete_workspace_branch(repo_path: &Path, branch_name: &str) -> Result<()> {
    crate::validate::validate_ref(branch_name)?;
    run_git(repo_path, &["branch", "-D", branch_name]).await
}

/// Prune orphan worktrees.
pub async fn prune_worktrees(repo_path: &Path) -> Result<()> {
    run_git(repo_path, &["worktree", "prune"]).await
}

// --- Internal helpers ---

async fn write_and_commit(worktree: &Path, files: &[FileWrite], message: &str) -> Result<()> {
    for f in files {
        if let Some(ref content) = f.content {
            // Write file
            let file_path = worktree.join(&f.path);
            if let Some(parent) = file_path.parent() {
                tokio::fs::create_dir_all(parent)
                    .await
                    .map_err(|e| DeltaError::Storage(format!("mkdir failed: {}", e)))?;
            }
            tokio::fs::write(&file_path, content)
                .await
                .map_err(|e| DeltaError::Storage(format!("write failed: {}", e)))?;
            run_git_in(worktree, &["add", "--", &f.path]).await?;
        } else {
            // Delete file
            run_git_in(worktree, &["rm", "--", &f.path]).await?;
        }
    }

    run_git_in(worktree, &["commit", "-m", message]).await
}

fn validate_author(name: &str, email: &str) -> Result<()> {
    if name.is_empty() || email.is_empty() {
        return Err(DeltaError::InvalidRef(
            "author name/email cannot be empty".into(),
        ));
    }
    for (label, value) in [("name", name), ("email", email)] {
        if value.starts_with('-') {
            return Err(DeltaError::InvalidRef(format!(
                "author {} cannot start with '-'",
                label
            )));
        }
        if value.contains('\0') || value.contains('\n') || value.contains('\r') {
            return Err(DeltaError::InvalidRef(format!(
                "author {} contains invalid characters",
                label
            )));
        }
    }
    Ok(())
}

/// Validate a file path for safety within a workspace.
fn validate_file_path(path: &str) -> Result<()> {
    if path.is_empty() {
        return Err(DeltaError::InvalidRef("file path cannot be empty".into()));
    }
    if path.contains('\0') {
        return Err(DeltaError::InvalidRef(
            "file path contains null bytes".into(),
        ));
    }
    if path.starts_with('/') {
        return Err(DeltaError::InvalidRef(
            "file path must not be absolute".into(),
        ));
    }
    for component in path.split('/') {
        if component == ".." {
            return Err(DeltaError::InvalidRef(
                "file path must not contain '..' components".into(),
            ));
        }
    }
    Ok(())
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
        let cmd = args.first().unwrap_or(&"");
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::error!("git {} failed: {}", cmd, stderr);
        return Err(DeltaError::Storage(format!("git {} failed", cmd)));
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
        let cmd = args.first().unwrap_or(&"");
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::error!("git {} failed: {}", cmd, stderr);
        return Err(DeltaError::Storage(format!("git {} failed", cmd)));
    }
    Ok(())
}

async fn run_git_output(repo_path: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| DeltaError::Storage(format!("git failed: {}", e)))?;

    if !output.status.success() {
        let cmd = args.first().unwrap_or(&"");
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::error!("git {} failed: {}", cmd, stderr);
        return Err(DeltaError::Storage(format!("git {} failed", cmd)));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_file_path_ok() {
        assert!(validate_file_path("src/main.rs").is_ok());
        assert!(validate_file_path("file.txt").is_ok());
        assert!(validate_file_path("a/b/c/d.rs").is_ok());
    }

    #[test]
    fn test_validate_file_path_rejects_traversal() {
        assert!(validate_file_path("../etc/passwd").is_err());
        assert!(validate_file_path("a/../b").is_err());
    }

    #[test]
    fn test_validate_file_path_rejects_absolute() {
        assert!(validate_file_path("/etc/passwd").is_err());
    }

    #[test]
    fn test_validate_file_path_rejects_null() {
        assert!(validate_file_path("a\0b").is_err());
    }

    #[test]
    fn test_validate_file_path_rejects_empty() {
        assert!(validate_file_path("").is_err());
    }
}
