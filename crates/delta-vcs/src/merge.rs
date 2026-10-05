//! Git merge execution for pull requests.

use delta_core::{DeltaError, Result};
use std::path::Path;
use std::process::Stdio;
use tokio::process::Command;

use crate::validate::validate_ref;

/// Validate author name/email to prevent git config injection.
fn validate_author(name: &str, email: &str) -> Result<()> {
    if name.is_empty() || email.is_empty() {
        return Err(DeltaError::InvalidRef(
            "author name/email cannot be empty".into(),
        ));
    }
    // Reject control characters and characters that could be interpreted as git options
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
    validate_author(author_name, author_email)?;

    let tmp_dir = tempfile::tempdir()
        .map_err(|e| DeltaError::Storage(format!("failed to create temp dir: {}", e)))?;
    let worktree_path = tmp_dir.path().join("merge-worktree");
    let worktree_str = worktree_path
        .to_str()
        .ok_or_else(|| DeltaError::Storage("worktree path is not valid UTF-8".into()))?;

    // Add worktree at the base branch
    run_git(repo_path, &["worktree", "add", worktree_str, base_branch]).await?;

    // The identity is passed per command: `git config` in a linked worktree
    // would write the shared repository config, racing with other merges.
    let identity = Identity {
        name: author_name,
        email: author_email,
    };
    let merge_result = do_merge(
        &worktree_path,
        base_branch,
        head_branch,
        strategy,
        merge_message,
        &identity,
    )
    .await;

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

/// Identity recorded on commits created by a merge.
struct Identity<'a> {
    name: &'a str,
    email: &'a str,
}

impl Identity<'_> {
    /// Environment making this identity the committer, and the author too
    /// when `author` is set (rebases keep the original authors).
    fn env(&self, author: bool) -> Vec<(&'static str, &str)> {
        let mut env = vec![
            ("GIT_COMMITTER_NAME", self.name),
            ("GIT_COMMITTER_EMAIL", self.email),
        ];
        if author {
            env.push(("GIT_AUTHOR_NAME", self.name));
            env.push(("GIT_AUTHOR_EMAIL", self.email));
        }
        env
    }
}

async fn do_merge(
    worktree: &Path,
    base_branch: &str,
    head_branch: &str,
    strategy: MergeMode,
    message: &str,
    identity: &Identity<'_>,
) -> Result<()> {
    let ours = identity.env(true);
    match strategy {
        MergeMode::Merge => {
            run_git_env(
                worktree,
                &["merge", "--no-ff", "-m", message, head_branch],
                &ours,
            )
            .await
        }
        MergeMode::Squash => {
            run_git_env(worktree, &["merge", "--squash", head_branch], &ours).await?;
            run_git_env(worktree, &["commit", "-m", message], &ours).await
        }
        MergeMode::Rebase => {
            // Replay the head branch's commits onto the base branch, then
            // fast-forward the base to the result. (Rebasing the checked-out
            // base onto the head would rewrite the base's own history.)
            let committer = identity.env(false);
            run_git_env(worktree, &["checkout", "--detach", head_branch], &committer).await?;
            run_git_env(worktree, &["rebase", base_branch], &committer).await?;
            let rebased = run_git_output(worktree, &["rev-parse", "HEAD"]).await?;
            run_git_env(worktree, &["checkout", base_branch], &committer).await?;
            run_git_env(
                worktree,
                &["merge", "--ff-only", rebased.trim()],
                &committer,
            )
            .await
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
        let cmd = args.first().unwrap_or(&"");
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::error!("git {} failed: {}", cmd, stderr);
        return Err(DeltaError::Storage(format!("git {} failed", cmd)));
    }
    Ok(())
}

async fn run_git_env(worktree: &Path, args: &[&str], env: &[(&str, &str)]) -> Result<()> {
    let output = Command::new("git")
        .args(args)
        .envs(env.iter().copied())
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
    fn test_validate_author_valid() {
        assert!(validate_author("Alice", "alice@example.com").is_ok());
        assert!(validate_author("Bob Smith", "bob@test.org").is_ok());
        assert!(validate_author("CI Bot", "ci@delta.local").is_ok());
    }

    #[test]
    fn test_validate_author_empty_name() {
        let err = validate_author("", "alice@example.com").unwrap_err();
        assert!(matches!(err, DeltaError::InvalidRef(_)));
    }

    #[test]
    fn test_validate_author_empty_email() {
        let err = validate_author("Alice", "").unwrap_err();
        assert!(matches!(err, DeltaError::InvalidRef(_)));
    }

    #[test]
    fn test_validate_author_leading_hyphen_name() {
        let err = validate_author("-Alice", "alice@example.com").unwrap_err();
        assert!(matches!(err, DeltaError::InvalidRef(_)));
    }

    #[test]
    fn test_validate_author_leading_hyphen_email() {
        let err = validate_author("Alice", "-alice@example.com").unwrap_err();
        assert!(matches!(err, DeltaError::InvalidRef(_)));
    }

    #[test]
    fn test_validate_author_null_byte() {
        assert!(validate_author("Ali\0ce", "alice@example.com").is_err());
        assert!(validate_author("Alice", "alice\0@example.com").is_err());
    }

    #[test]
    fn test_validate_author_newline() {
        assert!(validate_author("Alice\n", "alice@example.com").is_err());
        assert!(validate_author("Alice", "alice@example.com\n").is_err());
    }

    #[test]
    fn test_validate_author_carriage_return() {
        assert!(validate_author("Alice\r", "alice@example.com").is_err());
        assert!(validate_author("Alice", "alice@example.com\r").is_err());
    }
}
