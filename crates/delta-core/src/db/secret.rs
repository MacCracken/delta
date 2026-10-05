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

/// Fetch all secrets for a repo with their encrypted values (for pipeline execution).
pub async fn get_all_values(pool: &SqlitePool, repo_id: &str) -> Result<Vec<(String, String)>> {
    let rows = sqlx::query_as::<_, SecretValueRow>(
        "SELECT name, encrypted_value FROM repo_secrets WHERE repo_id = ? ORDER BY name",
    )
    .bind(repo_id)
    .fetch_all(pool)
    .await
    .map_err(|e| DeltaError::Storage(e.to_string()))?;

    Ok(rows
        .into_iter()
        .map(|r| (r.name, r.encrypted_value))
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
        return Err(DeltaError::RepoNotFound(format!(
            "secret '{}' not found",
            name
        )));
    }
    Ok(())
}

/// Re-encrypt legacy secrets (those without MAC tags) using the current format.
///
/// A value is left alone only if it verifies as the authenticated format.
/// Anything else is decrypted as the legacy format and re-encrypted — whatever
/// its length (legacy values of 32+ plaintext bytes are as long as an
/// authenticated value, so length alone can't tell them apart). Decrypting an
/// authenticated value under a wrong key that way yields random bytes, which
/// are rejected as invalid UTF-8, so a wrong key doesn't corrupt anything.
/// Returns the count of migrated secrets.
pub async fn re_encrypt_legacy(pool: &SqlitePool, encryption_passphrase: &str) -> Result<u64> {
    let key = crate::crypto::derive_key(encryption_passphrase);

    let rows =
        sqlx::query_as::<_, SecretValueWithIdRow>("SELECT id, encrypted_value FROM repo_secrets")
            .fetch_all(pool)
            .await
            .map_err(|e| DeltaError::Storage(e.to_string()))?;

    let mut count = 0u64;
    for row in rows {
        let Ok(raw) = hex::decode(&row.encrypted_value) else {
            continue;
        };
        // Authenticated format (>= 48 bytes with a valid tag): nothing to do.
        if raw.len() >= 48 && crate::crypto::decrypt(&key, &row.encrypted_value).is_ok() {
            continue;
        }
        // Secrets are text; random bytes from a wrong key almost never are.
        let printable = |p: &str| {
            p.chars()
                .all(|c| !c.is_control() || matches!(c, '\n' | '\r' | '\t'))
        };
        let plaintext =
            match crate::crypto::decrypt_legacy_unauthenticated(&key, &row.encrypted_value) {
                Ok(p) if printable(&p) => p,
                _ => {
                    tracing::warn!(
                        secret_id = %row.id,
                        "secret could not be decrypted (wrong secrets_key?); leaving it unchanged"
                    );
                    continue;
                }
            };
        let new_encrypted = crate::crypto::encrypt(&key, plaintext.as_bytes())?;
        let now = Utc::now().to_rfc3339();
        sqlx::query("UPDATE repo_secrets SET encrypted_value = ?, updated_at = ? WHERE id = ?")
            .bind(&new_encrypted)
            .bind(&now)
            .bind(&row.id)
            .execute(pool)
            .await
            .map_err(|e| DeltaError::Storage(e.to_string()))?;
        count += 1;
    }

    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_re_encrypt_legacy_handles_long_values() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        crate::db::run_migrations(&pool).await.unwrap();
        let user = crate::db::user::create(&pool, "u", "u@example.com", "pw", false)
            .await
            .unwrap();
        let repo = crate::db::repo::create(
            &pool,
            &user.id.to_string(),
            "r",
            None,
            crate::models::repo::Visibility::Private,
        )
        .await
        .unwrap();
        let repo_id = repo.id.to_string();
        let key = crate::crypto::derive_key("passphrase");

        // A 40-character token: as long as an authenticated value would be.
        let long = "ghp_0123456789abcdefghijklmnopqrstuvwxyz";
        let short = "hunter2";
        for (name, value) in [("LONG", long), ("SHORT", short)] {
            let legacy = crate::crypto::encrypt_legacy(&key, value.as_bytes());
            sqlx::query(
                "INSERT INTO repo_secrets (id, repo_id, name, encrypted_value, created_at, updated_at)
                 VALUES (?, ?, ?, ?, '', '')",
            )
            .bind(name)
            .bind(&repo_id)
            .bind(name)
            .bind(&legacy)
            .execute(&pool)
            .await
            .unwrap();
        }

        // A wrong key must not rewrite anything.
        assert_eq!(re_encrypt_legacy(&pool, "wrong").await.unwrap(), 0);
        assert_eq!(re_encrypt_legacy(&pool, "passphrase").await.unwrap(), 2);
        // Idempotent once migrated.
        assert_eq!(re_encrypt_legacy(&pool, "passphrase").await.unwrap(), 0);

        let values = get_all_values(&pool, &repo_id).await.unwrap();
        for (name, encrypted) in values {
            let expected = if name == "LONG" { long } else { short };
            assert_eq!(crate::crypto::decrypt(&key, &encrypted).unwrap(), expected);
        }
    }
}

#[derive(sqlx::FromRow)]
struct SecretValueWithIdRow {
    id: String,
    encrypted_value: String,
}

#[derive(sqlx::FromRow)]
struct SecretRow {
    id: String,
    repo_id: String,
    name: String,
    created_at: String,
    updated_at: String,
}

#[derive(sqlx::FromRow)]
struct SecretValueRow {
    name: String,
    encrypted_value: String,
}
