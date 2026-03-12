use crate::{DeltaError, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SigningKey {
    pub id: String,
    pub user_id: String,
    pub name: String,
    pub public_key_hex: String,
    pub created_at: String,
}

#[derive(sqlx::FromRow)]
struct SigningKeyRow {
    id: String,
    user_id: String,
    name: String,
    public_key_hex: String,
    created_at: String,
}

impl SigningKeyRow {
    fn into_key(self) -> SigningKey {
        SigningKey {
            id: self.id,
            user_id: self.user_id,
            name: self.name,
            public_key_hex: self.public_key_hex,
            created_at: self.created_at,
        }
    }
}

pub async fn add_signing_key(
    pool: &SqlitePool,
    user_id: &str,
    name: &str,
    public_key_hex: &str,
) -> Result<SigningKey> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();

    sqlx::query(
        "INSERT INTO user_signing_keys (id, user_id, name, public_key_hex, created_at) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(user_id)
    .bind(name)
    .bind(public_key_hex)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(|e| {
        if e.to_string().contains("UNIQUE") {
            DeltaError::Conflict(format!("signing key '{}' already exists", name))
        } else {
            DeltaError::Registry(e.to_string())
        }
    })?;

    Ok(SigningKey {
        id,
        user_id: user_id.to_string(),
        name: name.to_string(),
        public_key_hex: public_key_hex.to_string(),
        created_at: now,
    })
}

pub async fn list_signing_keys(pool: &SqlitePool, user_id: &str) -> Result<Vec<SigningKey>> {
    let rows = sqlx::query_as::<_, SigningKeyRow>(
        "SELECT * FROM user_signing_keys WHERE user_id = ? ORDER BY created_at DESC",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
    .map_err(|e| DeltaError::Registry(e.to_string()))?;
    Ok(rows.into_iter().map(|r| r.into_key()).collect())
}

pub async fn get_signing_key(pool: &SqlitePool, key_id: &str) -> Result<SigningKey> {
    sqlx::query_as::<_, SigningKeyRow>("SELECT * FROM user_signing_keys WHERE id = ?")
        .bind(key_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| DeltaError::Registry(e.to_string()))?
        .map(|r| r.into_key())
        .ok_or_else(|| DeltaError::Registry("signing key not found".into()))
}

pub async fn delete_signing_key(pool: &SqlitePool, key_id: &str, user_id: &str) -> Result<()> {
    let result = sqlx::query("DELETE FROM user_signing_keys WHERE id = ? AND user_id = ?")
        .bind(key_id)
        .bind(user_id)
        .execute(pool)
        .await
        .map_err(|e| DeltaError::Registry(e.to_string()))?;
    if result.rows_affected() == 0 {
        return Err(DeltaError::Registry("signing key not found".into()));
    }
    Ok(())
}

// --- Artifact Signatures ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactSignature {
    pub id: String,
    pub artifact_id: String,
    pub signer_key_id: String,
    pub signature_hex: String,
    pub created_at: String,
}

#[derive(sqlx::FromRow)]
struct SignatureRow {
    id: String,
    artifact_id: String,
    signer_key_id: String,
    signature_hex: String,
    created_at: String,
}

pub async fn add_signature(
    pool: &SqlitePool,
    artifact_id: &str,
    signer_key_id: &str,
    signature_hex: &str,
) -> Result<ArtifactSignature> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();

    sqlx::query(
        "INSERT INTO artifact_signatures (id, artifact_id, signer_key_id, signature_hex, created_at) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(artifact_id)
    .bind(signer_key_id)
    .bind(signature_hex)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(|e| {
        if e.to_string().contains("UNIQUE") {
            DeltaError::Conflict("artifact already signed with this key".into())
        } else {
            DeltaError::Registry(e.to_string())
        }
    })?;

    Ok(ArtifactSignature {
        id,
        artifact_id: artifact_id.to_string(),
        signer_key_id: signer_key_id.to_string(),
        signature_hex: signature_hex.to_string(),
        created_at: now,
    })
}

pub async fn get_signatures(
    pool: &SqlitePool,
    artifact_id: &str,
) -> Result<Vec<ArtifactSignature>> {
    let rows = sqlx::query_as::<_, SignatureRow>(
        "SELECT * FROM artifact_signatures WHERE artifact_id = ? ORDER BY created_at DESC",
    )
    .bind(artifact_id)
    .fetch_all(pool)
    .await
    .map_err(|e| DeltaError::Registry(e.to_string()))?;

    Ok(rows
        .into_iter()
        .map(|r| ArtifactSignature {
            id: r.id,
            artifact_id: r.artifact_id,
            signer_key_id: r.signer_key_id,
            signature_hex: r.signature_hex,
            created_at: r.created_at,
        })
        .collect())
}
