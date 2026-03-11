use crate::models::user::User;
use crate::{DeltaError, Result};
use chrono::Utc;
use sqlx::SqlitePool;
use uuid::Uuid;

/// Create a new user.
pub async fn create(
    pool: &SqlitePool,
    username: &str,
    email: &str,
    password_hash: &str,
    is_agent: bool,
) -> Result<User> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();

    sqlx::query(
        "INSERT INTO users (id, username, email, password_hash, is_agent, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(username)
    .bind(email)
    .bind(password_hash)
    .bind(is_agent)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(|e| {
        if e.to_string().contains("UNIQUE") {
            DeltaError::Conflict(format!("user '{}' already exists", username))
        } else {
            DeltaError::Storage(e.to_string())
        }
    })?;

    get_by_id(pool, &id).await
}

/// Get a user by ID.
pub async fn get_by_id(pool: &SqlitePool, id: &str) -> Result<User> {
    let row = sqlx::query_as::<_, UserRow>("SELECT * FROM users WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|e| DeltaError::Storage(e.to_string()))?
        .ok_or_else(|| DeltaError::AuthFailed("user not found".into()))?;

    Ok(row.into_user())
}

/// Get a user by username.
pub async fn get_by_username(pool: &SqlitePool, username: &str) -> Result<User> {
    let row = sqlx::query_as::<_, UserRow>("SELECT * FROM users WHERE username = ?")
        .bind(username)
        .fetch_optional(pool)
        .await
        .map_err(|e| DeltaError::Storage(e.to_string()))?
        .ok_or_else(|| DeltaError::AuthFailed("user not found".into()))?;

    Ok(row.into_user())
}

/// Get a user's password hash for verification.
pub async fn get_password_hash(pool: &SqlitePool, username: &str) -> Result<(String, String)> {
    let row = sqlx::query_as::<_, (String, String)>(
        "SELECT id, password_hash FROM users WHERE username = ?",
    )
    .bind(username)
    .fetch_optional(pool)
    .await
    .map_err(|e| DeltaError::Storage(e.to_string()))?
    .ok_or_else(|| DeltaError::AuthFailed("invalid credentials".into()))?;

    Ok(row)
}

/// Store an API token.
pub async fn create_token(
    pool: &SqlitePool,
    user_id: &str,
    name: &str,
    token_hash: &str,
    scopes: &str,
    expires_at: Option<&str>,
) -> Result<String> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();

    sqlx::query(
        "INSERT INTO api_tokens (id, user_id, name, token_hash, scopes, expires_at, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(user_id)
    .bind(name)
    .bind(token_hash)
    .bind(scopes)
    .bind(expires_at)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(|e| DeltaError::Storage(e.to_string()))?;

    Ok(id)
}

/// Look up a user by token hash. Returns None if expired.
pub async fn get_by_token_hash(pool: &SqlitePool, token_hash: &str) -> Result<Option<User>> {
    let row = sqlx::query_as::<_, UserRow>(
        "SELECT u.* FROM users u
         JOIN api_tokens t ON u.id = t.user_id
         WHERE t.token_hash = ?
         AND (t.expires_at IS NULL OR t.expires_at > datetime('now'))",
    )
    .bind(token_hash)
    .fetch_optional(pool)
    .await
    .map_err(|e| DeltaError::Storage(e.to_string()))?;

    if let Some(row) = row {
        // Update last_used_at
        let _ = sqlx::query(
            "UPDATE api_tokens SET last_used_at = datetime('now') WHERE token_hash = ?",
        )
        .bind(token_hash)
        .execute(pool)
        .await;

        Ok(Some(row.into_user()))
    } else {
        Ok(None)
    }
}

/// List API tokens for a user (without revealing hashes).
pub async fn list_tokens(pool: &SqlitePool, user_id: &str) -> Result<Vec<TokenInfo>> {
    let rows = sqlx::query_as::<_, TokenInfoRow>(
        "SELECT id, name, scopes, expires_at, last_used_at, created_at
         FROM api_tokens WHERE user_id = ? ORDER BY created_at DESC",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
    .map_err(|e| DeltaError::Storage(e.to_string()))?;

    Ok(rows
        .into_iter()
        .map(|r| TokenInfo {
            id: r.id,
            name: r.name,
            scopes: r.scopes,
            expires_at: r.expires_at,
            last_used_at: r.last_used_at,
            created_at: r.created_at,
        })
        .collect())
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct TokenInfo {
    pub id: String,
    pub name: String,
    pub scopes: String,
    pub expires_at: Option<String>,
    pub last_used_at: Option<String>,
    pub created_at: String,
}

#[derive(sqlx::FromRow)]
struct TokenInfoRow {
    id: String,
    name: String,
    scopes: String,
    expires_at: Option<String>,
    last_used_at: Option<String>,
    created_at: String,
}

/// Delete an API token.
pub async fn delete_token(pool: &SqlitePool, token_id: &str, user_id: &str) -> Result<()> {
    let result = sqlx::query("DELETE FROM api_tokens WHERE id = ? AND user_id = ?")
        .bind(token_id)
        .bind(user_id)
        .execute(pool)
        .await
        .map_err(|e| DeltaError::Storage(e.to_string()))?;

    if result.rows_affected() == 0 {
        return Err(DeltaError::AuthFailed("token not found".into()));
    }
    Ok(())
}

#[derive(sqlx::FromRow)]
struct UserRow {
    id: String,
    username: String,
    display_name: Option<String>,
    email: String,
    #[allow(dead_code)]
    password_hash: String,
    is_agent: bool,
    created_at: String,
    #[allow(dead_code)]
    updated_at: String,
}

impl UserRow {
    fn into_user(self) -> User {
        User {
            id: self.id.parse().unwrap_or_default(),
            username: self.username,
            display_name: self.display_name,
            email: self.email,
            is_agent: self.is_agent,
            created_at: self.created_at.parse().unwrap_or_default(),
        }
    }
}
