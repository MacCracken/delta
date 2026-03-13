//! Repository encryption key management.

use crate::{DeltaError, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoEncryptionKey {
    pub id: String,
    pub repo_id: String,
    pub user_id: String,
    pub encrypted_key: String,
    pub algorithm: String,
    pub created_at: String,
}

#[derive(sqlx::FromRow)]
struct KeyRow {
    id: String,
    repo_id: String,
    user_id: String,
    encrypted_key: String,
    algorithm: String,
    created_at: String,
}

pub async fn add_key(
    pool: &SqlitePool,
    repo_id: &str,
    user_id: &str,
    encrypted_key: &str,
    algorithm: &str,
) -> Result<RepoEncryptionKey> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();

    sqlx::query(
        "INSERT INTO repo_encryption_keys (id, repo_id, user_id, encrypted_key, algorithm, created_at) VALUES (?, ?, ?, ?, ?, ?)"
    )
    .bind(&id)
    .bind(repo_id)
    .bind(user_id)
    .bind(encrypted_key)
    .bind(algorithm)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(|e| {
        if e.to_string().contains("UNIQUE") {
            DeltaError::Conflict("encryption key already exists for this user/repo".into())
        } else {
            DeltaError::Storage(e.to_string())
        }
    })?;

    Ok(RepoEncryptionKey {
        id,
        repo_id: repo_id.to_string(),
        user_id: user_id.to_string(),
        encrypted_key: encrypted_key.to_string(),
        algorithm: algorithm.to_string(),
        created_at: now,
    })
}

pub async fn get_key(
    pool: &SqlitePool,
    repo_id: &str,
    user_id: &str,
) -> Result<RepoEncryptionKey> {
    sqlx::query_as::<_, KeyRow>(
        "SELECT * FROM repo_encryption_keys WHERE repo_id = ? AND user_id = ?",
    )
    .bind(repo_id)
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| DeltaError::Storage(e.to_string()))?
    .map(|r| RepoEncryptionKey {
        id: r.id,
        repo_id: r.repo_id,
        user_id: r.user_id,
        encrypted_key: r.encrypted_key,
        algorithm: r.algorithm,
        created_at: r.created_at,
    })
    .ok_or_else(|| DeltaError::NotFound("encryption key not found".into()))
}

pub async fn list_keys_for_repo(
    pool: &SqlitePool,
    repo_id: &str,
) -> Result<Vec<RepoEncryptionKey>> {
    let rows = sqlx::query_as::<_, KeyRow>(
        "SELECT * FROM repo_encryption_keys WHERE repo_id = ? ORDER BY created_at DESC",
    )
    .bind(repo_id)
    .fetch_all(pool)
    .await
    .map_err(|e| DeltaError::Storage(e.to_string()))?;

    Ok(rows
        .into_iter()
        .map(|r| RepoEncryptionKey {
            id: r.id,
            repo_id: r.repo_id,
            user_id: r.user_id,
            encrypted_key: r.encrypted_key,
            algorithm: r.algorithm,
            created_at: r.created_at,
        })
        .collect())
}

pub async fn delete_key(pool: &SqlitePool, id: &str) -> Result<()> {
    let result = sqlx::query("DELETE FROM repo_encryption_keys WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await
        .map_err(|e| DeltaError::Storage(e.to_string()))?;

    if result.rows_affected() == 0 {
        return Err(DeltaError::NotFound("encryption key not found".into()));
    }
    Ok(())
}

pub async fn set_repo_encrypted(pool: &SqlitePool, repo_id: &str, encrypted: bool) -> Result<()> {
    sqlx::query("UPDATE repositories SET encrypted = ? WHERE id = ?")
        .bind(encrypted)
        .bind(repo_id)
        .execute(pool)
        .await
        .map_err(|e| DeltaError::Storage(e.to_string()))?;
    Ok(())
}
