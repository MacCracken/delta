//! Workspace CRUD operations.

use crate::models::workspace::{Workspace, WorkspaceStatus};
use crate::{DeltaError, Result};
use chrono::Utc;
use sqlx::SqlitePool;
use uuid::Uuid;

pub struct CreateWorkspaceParams<'a> {
    pub repo_id: &'a str,
    pub creator_id: &'a str,
    pub name: &'a str,
    pub branch: &'a str,
    pub base_branch: &'a str,
    pub base_commit: &'a str,
    pub ttl_hours: i64,
}

pub async fn create(pool: &SqlitePool, params: CreateWorkspaceParams<'_>) -> Result<Workspace> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    let expires_at = (Utc::now() + chrono::Duration::hours(params.ttl_hours)).to_rfc3339();

    sqlx::query(
        "INSERT INTO workspaces (id, repo_id, creator_id, name, branch, base_branch, base_commit, status, ttl_hours, expires_at, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, 'active', ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(params.repo_id)
    .bind(params.creator_id)
    .bind(params.name)
    .bind(params.branch)
    .bind(params.base_branch)
    .bind(params.base_commit)
    .bind(params.ttl_hours)
    .bind(&expires_at)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(|e| {
        if e.to_string().contains("UNIQUE") {
            DeltaError::Conflict(format!(
                "workspace branch '{}' already exists",
                params.branch
            ))
        } else {
            DeltaError::Storage(e.to_string())
        }
    })?;

    get_by_id(pool, &id).await
}

pub async fn get_by_id(pool: &SqlitePool, id: &str) -> Result<Workspace> {
    let row = sqlx::query_as::<_, WorkspaceRow>("SELECT * FROM workspaces WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|e| DeltaError::Storage(e.to_string()))?
        .ok_or_else(|| DeltaError::NotFound("workspace not found".into()))?;
    Ok(row.into_workspace())
}

pub async fn list_for_repo(
    pool: &SqlitePool,
    repo_id: &str,
    status: Option<&str>,
    limit: i64,
) -> Result<Vec<Workspace>> {
    let rows = if let Some(s) = status {
        sqlx::query_as::<_, WorkspaceRow>(
            "SELECT * FROM workspaces WHERE repo_id = ? AND status = ? ORDER BY created_at DESC LIMIT ?",
        )
        .bind(repo_id)
        .bind(s)
        .bind(limit)
        .fetch_all(pool)
        .await
    } else {
        sqlx::query_as::<_, WorkspaceRow>(
            "SELECT * FROM workspaces WHERE repo_id = ? ORDER BY created_at DESC LIMIT ?",
        )
        .bind(repo_id)
        .bind(limit)
        .fetch_all(pool)
        .await
    }
    .map_err(|e| DeltaError::Storage(e.to_string()))?;

    Ok(rows.into_iter().map(|r| r.into_workspace()).collect())
}

pub async fn update_head_commit(
    pool: &SqlitePool,
    id: &str,
    head_commit: &str,
) -> Result<Workspace> {
    let now = Utc::now().to_rfc3339();
    sqlx::query("UPDATE workspaces SET head_commit = ?, updated_at = ? WHERE id = ?")
        .bind(head_commit)
        .bind(&now)
        .bind(id)
        .execute(pool)
        .await
        .map_err(|e| DeltaError::Storage(e.to_string()))?;
    get_by_id(pool, id).await
}

pub async fn update_status(
    pool: &SqlitePool,
    id: &str,
    status: WorkspaceStatus,
) -> Result<Workspace> {
    let now = Utc::now().to_rfc3339();
    sqlx::query("UPDATE workspaces SET status = ?, updated_at = ? WHERE id = ?")
        .bind(status.as_str())
        .bind(&now)
        .bind(id)
        .execute(pool)
        .await
        .map_err(|e| DeltaError::Storage(e.to_string()))?;
    get_by_id(pool, id).await
}

pub async fn extend_ttl(pool: &SqlitePool, id: &str, additional_hours: i64) -> Result<Workspace> {
    let ws = get_by_id(pool, id).await?;
    let new_expires = (ws.expires_at + chrono::Duration::hours(additional_hours)).to_rfc3339();
    let now = Utc::now().to_rfc3339();
    sqlx::query("UPDATE workspaces SET expires_at = ?, updated_at = ? WHERE id = ?")
        .bind(&new_expires)
        .bind(&now)
        .bind(id)
        .execute(pool)
        .await
        .map_err(|e| DeltaError::Storage(e.to_string()))?;
    get_by_id(pool, id).await
}

pub async fn list_expired(pool: &SqlitePool) -> Result<Vec<Workspace>> {
    let now = Utc::now().to_rfc3339();
    let rows = sqlx::query_as::<_, WorkspaceRow>(
        "SELECT * FROM workspaces WHERE status = 'active' AND expires_at <= ?",
    )
    .bind(&now)
    .fetch_all(pool)
    .await
    .map_err(|e| DeltaError::Storage(e.to_string()))?;
    Ok(rows.into_iter().map(|r| r.into_workspace()).collect())
}

// --- Row type ---

#[derive(sqlx::FromRow)]
struct WorkspaceRow {
    id: String,
    repo_id: String,
    creator_id: String,
    name: String,
    branch: String,
    base_branch: String,
    base_commit: String,
    head_commit: Option<String>,
    status: String,
    ttl_hours: i64,
    expires_at: String,
    created_at: String,
    updated_at: String,
}

impl WorkspaceRow {
    fn into_workspace(self) -> Workspace {
        Workspace {
            id: self.id.parse().unwrap_or_default(),
            repo_id: self.repo_id,
            creator_id: self.creator_id,
            name: self.name,
            branch: self.branch,
            base_branch: self.base_branch,
            base_commit: self.base_commit,
            head_commit: self.head_commit,
            status: WorkspaceStatus::parse(&self.status),
            ttl_hours: self.ttl_hours,
            expires_at: self.expires_at.parse().unwrap_or_default(),
            created_at: self.created_at.parse().unwrap_or_default(),
            updated_at: self.updated_at.parse().unwrap_or_default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_workspace_status_roundtrip() {
        for status in [
            WorkspaceStatus::Active,
            WorkspaceStatus::Closed,
            WorkspaceStatus::Expired,
        ] {
            assert_eq!(WorkspaceStatus::parse(status.as_str()), status);
        }
    }

    #[test]
    fn test_workspace_status_parse_unknown() {
        assert_eq!(WorkspaceStatus::parse("unknown"), WorkspaceStatus::Active);
    }
}
