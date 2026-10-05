//! Temporary working-tree checkouts of hosted repositories, used by CI.

use delta_core::{DeltaError, Result};
use std::path::{Path, PathBuf};
use tempfile::TempDir;
use tokio::process::Command;

/// A commit checked out into a temporary directory, removed on drop.
///
/// Layout: `<root>/src` holds the working tree and `<root>/home` is an
/// empty directory usable as `$HOME` for build steps.
#[derive(Debug)]
pub struct Checkout {
    root: TempDir,
}

impl Checkout {
    /// The working tree.
    pub fn work_dir(&self) -> PathBuf {
        self.root.path().join("src")
    }

    /// A scratch home directory for processes run in the checkout.
    pub fn home_dir(&self) -> PathBuf {
        self.root.path().join("home")
    }
}

/// Check out `commit` from the bare repository at `repo_path`.
///
/// Objects are copied rather than hard-linked, and the clone is a separate
/// repository, so nothing done inside the checkout can modify the hosted
/// repository (its refs, hooks or objects).
pub async fn checkout_commit(repo_path: &Path, commit: &str) -> Result<Checkout> {
    if !matches!(commit.len(), 40 | 64) || !commit.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(DeltaError::InvalidRef(format!(
            "invalid commit id: {commit}"
        )));
    }

    let root = tempfile::Builder::new().prefix("delta-ci-").tempdir()?;
    // Container steps run as an unprivileged user and must be able to read
    // the checkout. (The service itself runs with a private /tmp.)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o755))?;
    }
    let checkout = Checkout { root };
    std::fs::create_dir(checkout.home_dir())?;

    let work_dir = checkout.work_dir();
    run_git(
        Command::new("git")
            .args(["clone", "--quiet", "--no-hardlinks", "--no-checkout", "--"])
            .arg(repo_path)
            .arg(&work_dir),
    )
    .await?;
    run_git(
        Command::new("git")
            .arg("-C")
            .arg(&work_dir)
            .args(["checkout", "--quiet", "--detach", commit]),
    )
    .await?;
    Ok(checkout)
}

async fn run_git(command: &mut Command) -> Result<()> {
    let output = command
        .kill_on_drop(true)
        .output()
        .await
        .map_err(|e| DeltaError::Storage(format!("failed to run git: {e}")))?;
    if !output.status.success() {
        return Err(DeltaError::Storage(format!(
            "git checkout failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git(dir: &Path, args: &[&str]) -> String {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    }

    #[tokio::test]
    async fn test_checkout_commit_is_isolated_from_hosted_repo() {
        let tmp = tempfile::tempdir().unwrap();
        let bare = tmp.path().join("repo.git");
        let work = tmp.path().join("work");
        std::fs::create_dir_all(&work).unwrap();
        git(
            tmp.path(),
            &["init", "-q", "--bare", bare.to_str().unwrap()],
        );
        git(&work, &["init", "-q", "-b", "main"]);
        git(&work, &["config", "user.email", "t@example.com"]);
        git(&work, &["config", "user.name", "t"]);
        std::fs::create_dir_all(work.join(".delta/workflows")).unwrap();
        std::fs::write(work.join(".delta/workflows/ci.toml"), "name = \"ci\"\n").unwrap();
        git(&work, &["add", "."]);
        git(&work, &["commit", "-q", "-m", "init"]);
        git(&work, &["push", "-q", bare.to_str().unwrap(), "main"]);
        let sha = git(&work, &["rev-parse", "HEAD"]);

        let checkout = checkout_commit(&bare, &sha).await.unwrap();
        assert!(
            checkout
                .work_dir()
                .join(".delta/workflows/ci.toml")
                .exists()
        );
        assert!(checkout.home_dir().is_dir());

        // Writing hooks in the checkout must not touch the hosted repo.
        std::fs::write(checkout.work_dir().join(".git/hooks/post-receive"), "x").unwrap();
        assert!(!bare.join("hooks/post-receive").exists());

        let root = checkout.work_dir().parent().unwrap().to_path_buf();
        drop(checkout);
        assert!(!root.exists());
    }

    #[tokio::test]
    async fn test_checkout_commit_rejects_non_sha() {
        let tmp = tempfile::tempdir().unwrap();
        for bad in ["main", "--upload-pack=x", "", "zz"] {
            assert!(checkout_commit(tmp.path(), bad).await.is_err());
        }
    }
}
