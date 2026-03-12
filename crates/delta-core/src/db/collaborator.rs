use crate::models::collaborator::{Collaborator, CollaboratorRole};
use crate::{DeltaError, Result};
use chrono::Utc;
use sqlx::SqlitePool;
use uuid::Uuid;

/// Add or update a collaborator on a repository.
pub async fn set(
    pool: &SqlitePool,
    repo_id: &str,
    user_id: &str,
    role: CollaboratorRole,
) -> Result<Collaborator> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    let role_str = role.as_str();

    sqlx::query(
        "INSERT INTO repository_collaborators (id, repo_id, user_id, role, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?)
         ON CONFLICT(repo_id, user_id) DO UPDATE SET role = excluded.role, updated_at = excluded.updated_at",
    )
    .bind(&id)
    .bind(repo_id)
    .bind(user_id)
    .bind(role_str)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(|e| DeltaError::Storage(e.to_string()))?;

    get(pool, repo_id, user_id).await
}

/// Get a specific collaborator record.
pub async fn get(pool: &SqlitePool, repo_id: &str, user_id: &str) -> Result<Collaborator> {
    let row = sqlx::query_as::<_, CollabRow>(
        "SELECT * FROM repository_collaborators WHERE repo_id = ? AND user_id = ?",
    )
    .bind(repo_id)
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| DeltaError::Storage(e.to_string()))?
    .ok_or_else(|| DeltaError::AuthzDenied("not a collaborator".into()))?;

    Ok(row.into_collaborator())
}

/// List all collaborators for a repository.
pub async fn list_for_repo(pool: &SqlitePool, repo_id: &str) -> Result<Vec<Collaborator>> {
    let rows = sqlx::query_as::<_, CollabRow>(
        "SELECT * FROM repository_collaborators WHERE repo_id = ? ORDER BY created_at",
    )
    .bind(repo_id)
    .fetch_all(pool)
    .await
    .map_err(|e| DeltaError::Storage(e.to_string()))?;

    Ok(rows.into_iter().map(|r| r.into_collaborator()).collect())
}

/// Remove a collaborator.
pub async fn remove(pool: &SqlitePool, repo_id: &str, user_id: &str) -> Result<()> {
    let result =
        sqlx::query("DELETE FROM repository_collaborators WHERE repo_id = ? AND user_id = ?")
            .bind(repo_id)
            .bind(user_id)
            .execute(pool)
            .await
            .map_err(|e| DeltaError::Storage(e.to_string()))?;

    if result.rows_affected() == 0 {
        return Err(DeltaError::AuthzDenied("collaborator not found".into()));
    }
    Ok(())
}

/// Get the role a user has on a repository (None if not a collaborator).
pub async fn get_role(
    pool: &SqlitePool,
    repo_id: &str,
    user_id: &str,
) -> Result<Option<CollaboratorRole>> {
    let row = sqlx::query_as::<_, (String,)>(
        "SELECT role FROM repository_collaborators WHERE repo_id = ? AND user_id = ?",
    )
    .bind(repo_id)
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| DeltaError::Storage(e.to_string()))?;

    Ok(row.and_then(|(r,)| CollaboratorRole::parse(&r)))
}

#[derive(sqlx::FromRow)]
struct CollabRow {
    id: String,
    repo_id: String,
    user_id: String,
    role: String,
    created_at: String,
    updated_at: String,
}

impl CollabRow {
    fn into_collaborator(self) -> Collaborator {
        Collaborator {
            id: self.id.parse().unwrap_or_default(),
            repo_id: self.repo_id.parse().unwrap_or_default(),
            user_id: self.user_id.parse().unwrap_or_default(),
            role: CollaboratorRole::parse(&self.role).unwrap_or(CollaboratorRole::Read),
            created_at: self.created_at.parse().unwrap_or_default(),
            updated_at: self.updated_at.parse().unwrap_or_default(),
        }
    }
}
