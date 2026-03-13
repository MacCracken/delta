//! Federation instance registry.

use crate::{DeltaError, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FederationInstance {
    pub id: String,
    pub url: String,
    pub name: Option<String>,
    pub public_key: Option<String>,
    pub trusted: bool,
    pub last_seen_at: Option<String>,
    pub created_at: String,
}

#[derive(sqlx::FromRow)]
struct InstanceRow {
    id: String,
    url: String,
    name: Option<String>,
    public_key: Option<String>,
    trusted: bool,
    last_seen_at: Option<String>,
    created_at: String,
}

impl InstanceRow {
    fn into_instance(self) -> FederationInstance {
        FederationInstance {
            id: self.id,
            url: self.url,
            name: self.name,
            public_key: self.public_key,
            trusted: self.trusted,
            last_seen_at: self.last_seen_at,
            created_at: self.created_at,
        }
    }
}

pub async fn add_instance(
    pool: &SqlitePool,
    url: &str,
    name: Option<&str>,
    public_key: Option<&str>,
    trusted: bool,
) -> Result<FederationInstance> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();

    sqlx::query(
        "INSERT INTO federation_instances (id, url, name, public_key, trusted, created_at) VALUES (?, ?, ?, ?, ?, ?)"
    )
    .bind(&id)
    .bind(url)
    .bind(name)
    .bind(public_key)
    .bind(trusted)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(|e| {
        if e.to_string().contains("UNIQUE") {
            DeltaError::Conflict(format!("federation instance '{}' already registered", url))
        } else {
            DeltaError::Storage(e.to_string())
        }
    })?;

    Ok(FederationInstance {
        id,
        url: url.to_string(),
        name: name.map(|s| s.to_string()),
        public_key: public_key.map(|s| s.to_string()),
        trusted,
        last_seen_at: None,
        created_at: now,
    })
}

pub async fn list_instances(pool: &SqlitePool) -> Result<Vec<FederationInstance>> {
    let rows = sqlx::query_as::<_, InstanceRow>(
        "SELECT * FROM federation_instances ORDER BY created_at DESC",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| DeltaError::Storage(e.to_string()))?;

    Ok(rows.into_iter().map(|r| r.into_instance()).collect())
}

pub async fn get_instance(pool: &SqlitePool, id: &str) -> Result<FederationInstance> {
    sqlx::query_as::<_, InstanceRow>("SELECT * FROM federation_instances WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|e| DeltaError::Storage(e.to_string()))?
        .map(|r| r.into_instance())
        .ok_or_else(|| DeltaError::NotFound("federation instance not found".into()))
}

pub async fn get_instance_by_url(pool: &SqlitePool, url: &str) -> Result<FederationInstance> {
    sqlx::query_as::<_, InstanceRow>("SELECT * FROM federation_instances WHERE url = ?")
        .bind(url)
        .fetch_optional(pool)
        .await
        .map_err(|e| DeltaError::Storage(e.to_string()))?
        .map(|r| r.into_instance())
        .ok_or_else(|| DeltaError::NotFound("federation instance not found".into()))
}

pub async fn update_trust(pool: &SqlitePool, id: &str, trusted: bool) -> Result<()> {
    let result = sqlx::query("UPDATE federation_instances SET trusted = ? WHERE id = ?")
        .bind(trusted)
        .bind(id)
        .execute(pool)
        .await
        .map_err(|e| DeltaError::Storage(e.to_string()))?;

    if result.rows_affected() == 0 {
        return Err(DeltaError::NotFound(
            "federation instance not found".into(),
        ));
    }
    Ok(())
}

pub async fn update_last_seen(pool: &SqlitePool, id: &str) -> Result<()> {
    let now = Utc::now().to_rfc3339();
    sqlx::query("UPDATE federation_instances SET last_seen_at = ? WHERE id = ?")
        .bind(&now)
        .bind(id)
        .execute(pool)
        .await
        .map_err(|e| DeltaError::Storage(e.to_string()))?;
    Ok(())
}

pub async fn delete_instance(pool: &SqlitePool, id: &str) -> Result<()> {
    let result = sqlx::query("DELETE FROM federation_instances WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await
        .map_err(|e| DeltaError::Storage(e.to_string()))?;

    if result.rows_affected() == 0 {
        return Err(DeltaError::NotFound(
            "federation instance not found".into(),
        ));
    }
    Ok(())
}
