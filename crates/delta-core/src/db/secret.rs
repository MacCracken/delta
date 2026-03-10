use crate::{DeltaError, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoSecret {
    pub id: String,
    pub repo_id: String,
    pub name: String,
    pub created_at: String,
    pub updated_at: String,
}

pub async fn set(
    pool: &SqlitePool,
    repo_id: &str,
    name: &str,
    encrypted_value: &str,
) -> Result<RepoSecret> {
    let now = Utc::now().to_rfc3339();
    let id = Uuid::new_v4().to_string();

    // Atomic upsert via INSERT ON CONFLICT
    sqlx::query(
        "INSERT INTO repo_secrets (id, repo_id, name, encrypted_value, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?)
         ON CONFLICT(repo_id, name) DO UPDATE SET
           encrypted_value = excluded.encrypted_value,
           updated_at = excluded.updated_at",
    )
    .bind(&id)
    .bind(repo_id)
    .bind(name)
    .bind(encrypted_value)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(|e| DeltaError::Storage(e.to_string()))?;

    list(pool, repo_id)
        .await?
        .into_iter()
        .find(|s| s.name == name)
        .ok_or_else(|| DeltaError::Storage("failed to retrieve secret".into()))
}

pub async fn list(pool: &SqlitePool, repo_id: &str) -> Result<Vec<RepoSecret>> {
    let rows = sqlx::query_as::<_, SecretRow>(
        "SELECT id, repo_id, name, created_at, updated_at FROM repo_secrets WHERE repo_id = ? ORDER BY name",
    )
    .bind(repo_id)
    .fetch_all(pool)
    .await
    .map_err(|e| DeltaError::Storage(e.to_string()))?;

    Ok(rows
        .into_iter()
        .map(|r| RepoSecret {
            id: r.id,
            repo_id: r.repo_id,
            name: r.name,
            created_at: r.created_at,
            updated_at: r.updated_at,
        })
        .collect())
}

pub async fn delete(pool: &SqlitePool, repo_id: &str, name: &str) -> Result<()> {
    let result = sqlx::query("DELETE FROM repo_secrets WHERE repo_id = ? AND name = ?")
        .bind(repo_id)
        .bind(name)
        .execute(pool)
        .await
        .map_err(|e| DeltaError::Storage(e.to_string()))?;

    if result.rows_affected() == 0 {
        return Err(DeltaError::RepoNotFound(format!("secret '{}' not found", name)));
    }
    Ok(())
}

#[derive(sqlx::FromRow)]
struct SecretRow {
    id: String,
    repo_id: String,
    name: String,
    created_at: String,
    updated_at: String,
}
