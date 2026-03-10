//! Reference (branch/tag) management via gix.

use delta_core::{DeltaError, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchInfo {
    pub name: String,
    pub commit_id: String,
    pub is_default: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagInfo {
    pub name: String,
    pub target_id: String,
}

/// List all branches in a bare repository.
pub fn list_branches(repo_path: &Path) -> Result<Vec<BranchInfo>> {
    let repo = gix::open(repo_path)
        .map_err(|e| DeltaError::Storage(format!("failed to open repo: {}", e)))?;

    let head_ref = repo.head_ref().ok().flatten();
    let head_name = head_ref.as_ref().map(|r| r.name().shorten().to_string());

    let refs = repo
        .references()
        .map_err(|e| DeltaError::Storage(e.to_string()))?;

    let branches = refs
        .local_branches()
        .map_err(|e| DeltaError::Storage(e.to_string()))?;

    let mut result = Vec::new();
    for reference in branches.flatten() {
        let name = reference.name().shorten().to_string();
        let commit_id = reference.id().to_hex().to_string();
        let is_default = head_name.as_deref() == Some(name.as_str());
        result.push(BranchInfo {
            name,
            commit_id,
            is_default,
        });
    }

    Ok(result)
}

/// List all tags in a bare repository.
pub fn list_tags(repo_path: &Path) -> Result<Vec<TagInfo>> {
    let repo = gix::open(repo_path)
        .map_err(|e| DeltaError::Storage(format!("failed to open repo: {}", e)))?;

    let refs = repo
        .references()
        .map_err(|e| DeltaError::Storage(e.to_string()))?;

    let tags = refs
        .tags()
        .map_err(|e| DeltaError::Storage(e.to_string()))?;

    let mut result = Vec::new();
    for reference in tags.flatten() {
        let name = reference.name().shorten().to_string();
        let target_id = reference.id().to_hex().to_string();
        result.push(TagInfo { name, target_id });
    }

    Ok(result)
}

/// Get the branch name that HEAD points to (e.g. "main").
/// Returns None for empty repos or detached HEAD.
pub fn head_branch(repo_path: &Path) -> Option<String> {
    let repo = gix::open(repo_path).ok()?;
    let head_ref = repo.head_ref().ok()??;
    Some(head_ref.name().shorten().to_string())
}

/// Get the commit ID that HEAD points to, if any.
pub fn head_commit(repo_path: &Path) -> Result<Option<String>> {
    let repo = gix::open(repo_path)
        .map_err(|e| DeltaError::Storage(format!("failed to open repo: {}", e)))?;

    match repo.head_commit() {
        Ok(commit) => Ok(Some(commit.id().to_hex().to_string())),
        Err(_) => Ok(None), // Empty repo, no commits yet
    }
}
