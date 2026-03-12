use crate::models::ssh_key::SshKey;
use crate::{DeltaError, Result};
use chrono::Utc;
use sqlx::SqlitePool;
use uuid::Uuid;

/// Add a new SSH public key for a user.
pub async fn add(
    pool: &SqlitePool,
    user_id: &str,
    name: &str,
    public_key: &str,
    fingerprint: &str,
) -> Result<SshKey> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();

    sqlx::query(
        "INSERT INTO ssh_keys (id, user_id, name, public_key, fingerprint, created_at)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(user_id)
    .bind(name)
    .bind(public_key)
    .bind(fingerprint)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(|e| {
        if e.to_string().contains("UNIQUE") {
            DeltaError::Conflict("SSH key already registered".into())
        } else {
            DeltaError::Storage(e.to_string())
        }
    })?;

    get_by_id(pool, &id).await
}

/// Get an SSH key by ID.
pub async fn get_by_id(pool: &SqlitePool, id: &str) -> Result<SshKey> {
    let row = sqlx::query_as::<_, SshKeyRow>("SELECT * FROM ssh_keys WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|e| DeltaError::Storage(e.to_string()))?
        .ok_or_else(|| DeltaError::Storage("SSH key not found".into()))?;

    Ok(row.into_key())
}

/// Find the user who owns a given SSH public key fingerprint.
pub async fn get_user_by_fingerprint(
    pool: &SqlitePool,
    fingerprint: &str,
) -> Result<Option<(String, String)>> {
    let row = sqlx::query_as::<_, (String, String)>(
        "SELECT u.id, u.username FROM users u
         JOIN ssh_keys k ON k.user_id = u.id
         WHERE k.fingerprint = ?",
    )
    .bind(fingerprint)
    .fetch_optional(pool)
    .await
    .map_err(|e| DeltaError::Storage(e.to_string()))?;

    Ok(row)
}

/// List all SSH keys for a user.
pub async fn list_by_user(pool: &SqlitePool, user_id: &str) -> Result<Vec<SshKey>> {
    let rows = sqlx::query_as::<_, SshKeyRow>(
        "SELECT * FROM ssh_keys WHERE user_id = ? ORDER BY created_at DESC",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
    .map_err(|e| DeltaError::Storage(e.to_string()))?;

    Ok(rows.into_iter().map(|r| r.into_key()).collect())
}

/// Delete an SSH key (must belong to the given user).
pub async fn delete(pool: &SqlitePool, id: &str, user_id: &str) -> Result<()> {
    let result = sqlx::query("DELETE FROM ssh_keys WHERE id = ? AND user_id = ?")
        .bind(id)
        .bind(user_id)
        .execute(pool)
        .await
        .map_err(|e| DeltaError::Storage(e.to_string()))?;

    if result.rows_affected() == 0 {
        return Err(DeltaError::Storage("SSH key not found".into()));
    }
    Ok(())
}

#[derive(sqlx::FromRow)]
struct SshKeyRow {
    id: String,
    user_id: String,
    name: String,
    public_key: String,
    fingerprint: String,
    created_at: String,
}

impl SshKeyRow {
    fn into_key(self) -> SshKey {
        SshKey {
            id: self.id.parse().unwrap_or_default(),
            user_id: self.user_id.parse().unwrap_or_default(),
            name: self.name,
            public_key: self.public_key,
            fingerprint: self.fingerprint,
            created_at: self.created_at.parse().unwrap_or_default(),
        }
    }
}
