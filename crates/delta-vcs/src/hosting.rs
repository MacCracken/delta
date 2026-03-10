use delta_core::{DeltaError, Result};
use std::path::PathBuf;

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
    pub fn repo_path(&self, owner: &str, name: &str) -> PathBuf {
        self.repos_dir.join(owner).join(format!("{}.git", name))
    }

    /// Initialize a new bare repository.
    pub fn init_bare(&self, owner: &str, name: &str) -> Result<PathBuf> {
        let path = self.repo_path(owner, name);
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
        self.repo_path(owner, name).exists()
    }

    /// Delete a repository from disk.
    pub fn delete(&self, owner: &str, name: &str) -> Result<()> {
        let path = self.repo_path(owner, name);
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
