use delta_core::{DeltaError, Result};
use std::path::PathBuf;

use crate::validate::validate_name;

/// Manages bare git repositories on disk.
pub struct RepoHost {
    repos_dir: PathBuf,
}

impl RepoHost {
    pub fn new(repos_dir: impl Into<PathBuf>) -> Self {
        Self {
            repos_dir: repos_dir.into(),
        }
    }

    /// Returns the on-disk path for a repository.
    /// Validates inputs to prevent path traversal.
    pub fn repo_path(&self, owner: &str, name: &str) -> Result<PathBuf> {
        validate_name(owner)?;
        validate_name(name)?;
        Ok(self.repos_dir.join(owner).join(format!("{}.git", name)))
    }

    /// Initialize a new bare repository.
    pub fn init_bare(&self, owner: &str, name: &str) -> Result<PathBuf> {
        let path = self.repo_path(owner, name)?;
        if path.exists() {
            return Err(DeltaError::Conflict(format!(
                "repository {}/{} already exists",
                owner, name
            )));
        }
        std::fs::create_dir_all(&path)?;
        gix::init_bare(&path).map_err(|e: gix::init::Error| DeltaError::Storage(e.to_string()))?;
        tracing::info!(owner, name, "initialized bare repository");
        Ok(path)
    }

    /// Check if a repository exists on disk.
    pub fn exists(&self, owner: &str, name: &str) -> bool {
        self.repo_path(owner, name)
            .map(|p| p.exists())
            .unwrap_or(false)
    }

    /// Clone a bare repository (used for forking).
    pub fn clone_bare(
        &self,
        src_owner: &str,
        src_name: &str,
        dst_owner: &str,
        dst_name: &str,
    ) -> Result<PathBuf> {
        let src = self.repo_path(src_owner, src_name)?;
        if !src.exists() {
            return Err(DeltaError::RepoNotFound(format!(
                "{}/{}",
                src_owner, src_name
            )));
        }

        let dst = self.repo_path(dst_owner, dst_name)?;
        if dst.exists() {
            return Err(DeltaError::Conflict(format!(
                "repository {}/{} already exists",
                dst_owner, dst_name
            )));
        }

        // Ensure parent directory exists
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let output = std::process::Command::new("git")
            .args(["clone", "--bare"])
            .arg(&src)
            .arg(&dst)
            .output()
            .map_err(|e| DeltaError::Storage(format!("failed to run git clone: {}", e)))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            tracing::error!("git clone --bare failed: {}", stderr);
            return Err(DeltaError::Storage("git clone --bare failed".into()));
        }

        tracing::info!(
            src_owner,
            src_name,
            dst_owner,
            dst_name,
            "cloned bare repository (fork)"
        );
        Ok(dst)
    }

    /// Delete a repository from disk.
    pub fn delete(&self, owner: &str, name: &str) -> Result<()> {
        let path = self.repo_path(owner, name)?;
        if !path.exists() {
            return Err(DeltaError::RepoNotFound(format!("{}/{}", owner, name)));
        }
        std::fs::remove_dir_all(&path)?;
        tracing::info!(owner, name, "deleted repository");
        Ok(())
    }

    /// List all repositories for an owner.
    pub fn list_repos(&self, owner: &str) -> Result<Vec<String>> {
        let owner_dir = self.repos_dir.join(owner);
        if !owner_dir.exists() {
            return Ok(vec![]);
        }
        let mut repos = Vec::new();
        for entry in std::fs::read_dir(&owner_dir)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().to_string();
            if name.ends_with(".git") {
                repos.push(name.trim_end_matches(".git").to_string());
            }
        }
        Ok(repos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_repo_path_valid() {
        let tmp = tempfile::tempdir().unwrap();
        let host = RepoHost::new(tmp.path());
        let path = host.repo_path("alice", "myrepo").unwrap();
        assert!(path.ends_with("alice/myrepo.git"));
    }

    #[test]
    fn test_repo_path_rejects_traversal() {
        let tmp = tempfile::tempdir().unwrap();
        let host = RepoHost::new(tmp.path());
        assert!(host.repo_path("..", "myrepo").is_err());
        assert!(host.repo_path("alice", "..").is_err());
        assert!(host.repo_path("alice/../bob", "repo").is_err());
    }

    #[test]
    fn test_init_bare_and_exists() {
        let tmp = tempfile::tempdir().unwrap();
        let host = RepoHost::new(tmp.path());

        assert!(!host.exists("alice", "myrepo"));

        host.init_bare("alice", "myrepo").unwrap();

        assert!(host.exists("alice", "myrepo"));
    }

    #[test]
    fn test_init_bare_duplicate() {
        let tmp = tempfile::tempdir().unwrap();
        let host = RepoHost::new(tmp.path());

        host.init_bare("alice", "firstrepo").unwrap();
        let result = host.init_bare("alice", "firstrepo");
        assert!(result.is_err());
    }

    #[test]
    fn test_delete_repo() {
        let tmp = tempfile::tempdir().unwrap();
        let host = RepoHost::new(tmp.path());

        host.init_bare("alice", "todelete").unwrap();
        assert!(host.exists("alice", "todelete"));

        host.delete("alice", "todelete").unwrap();
        assert!(!host.exists("alice", "todelete"));
    }

    #[test]
    fn test_delete_nonexistent() {
        let tmp = tempfile::tempdir().unwrap();
        let host = RepoHost::new(tmp.path());

        let result = host.delete("alice", "nonexistent");
        assert!(result.is_err());
    }

    #[test]
    fn test_list_repos_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let host = RepoHost::new(tmp.path());

        let repos = host.list_repos("alice").unwrap();
        assert!(repos.is_empty());
    }

    #[test]
    fn test_list_repos() {
        let tmp = tempfile::tempdir().unwrap();
        let host = RepoHost::new(tmp.path());

        host.init_bare("alice", "repo1").unwrap();
        host.init_bare("alice", "repo2").unwrap();
        host.init_bare("bob", "other").unwrap();

        let mut repos = host.list_repos("alice").unwrap();
        repos.sort();
        assert_eq!(repos, vec!["repo1", "repo2"]);

        let repos = host.list_repos("bob").unwrap();
        assert_eq!(repos, vec!["other"]);
    }

    #[test]
    fn test_clone_bare() {
        let tmp = tempfile::tempdir().unwrap();
        let host = RepoHost::new(tmp.path());

        host.init_bare("alice", "source").unwrap();
        host.clone_bare("alice", "source", "bob", "fork").unwrap();

        assert!(host.exists("bob", "fork"));
    }

    #[test]
    fn test_clone_bare_src_not_found() {
        let tmp = tempfile::tempdir().unwrap();
        let host = RepoHost::new(tmp.path());

        let result = host.clone_bare("alice", "nope", "bob", "fork");
        assert!(result.is_err());
    }

    #[test]
    fn test_clone_bare_dst_exists() {
        let tmp = tempfile::tempdir().unwrap();
        let host = RepoHost::new(tmp.path());

        host.init_bare("alice", "source").unwrap();
        host.init_bare("bob", "existing").unwrap();

        let result = host.clone_bare("alice", "source", "bob", "existing");
        assert!(result.is_err());
    }
}
