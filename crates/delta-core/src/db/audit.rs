use crate::{DeltaError, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub id: String,
    pub user_id: Option<String>,
    pub action: String,
    pub resource_type: String,
    pub resource_id: Option<String>,
    pub details: Option<String>,
    pub ip_address: Option<String>,
    pub created_at: String,
}

pub async fn log(
    pool: &SqlitePool,
    user_id: Option<&str>,
    action: &str,
    resource_type: &str,
    resource_id: Option<&str>,
    details: Option<&str>,
    ip_address: Option<&str>,
) -> Result<()> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();

    sqlx::query(
        "INSERT INTO audit_log (id, user_id, action, resource_type, resource_id, details, ip_address, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(user_id)
    .bind(action)
    .bind(resource_type)
    .bind(resource_id)
    .bind(details)
    .bind(ip_address)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(|e| DeltaError::Storage(e.to_string()))?;

    Ok(())
}

pub async fn list(
    pool: &SqlitePool,
    user_id: Option<&str>,
    resource_type: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<Vec<AuditEntry>> {
    let rows = match (user_id, resource_type) {
        (Some(uid), Some(rt)) => {
            sqlx::query_as::<_, AuditRow>(
                "SELECT * FROM audit_log WHERE user_id = ? AND resource_type = ? ORDER BY created_at DESC LIMIT ? OFFSET ?",
            )
            .bind(uid)
            .bind(rt)
            .bind(limit)
            .bind(offset)
            .fetch_all(pool)
            .await
        }
        (Some(uid), None) => {
            sqlx::query_as::<_, AuditRow>(
                "SELECT * FROM audit_log WHERE user_id = ? ORDER BY created_at DESC LIMIT ? OFFSET ?",
            )
            .bind(uid)
            .bind(limit)
            .bind(offset)
            .fetch_all(pool)
            .await
        }
        (None, Some(rt)) => {
            sqlx::query_as::<_, AuditRow>(
                "SELECT * FROM audit_log WHERE resource_type = ? ORDER BY created_at DESC LIMIT ? OFFSET ?",
            )
            .bind(rt)
            .bind(limit)
            .bind(offset)
            .fetch_all(pool)
            .await
        }
        (None, None) => {
            sqlx::query_as::<_, AuditRow>(
                "SELECT * FROM audit_log ORDER BY created_at DESC LIMIT ? OFFSET ?",
            )
            .bind(limit)
            .bind(offset)
            .fetch_all(pool)
            .await
        }
    }
    .map_err(|e| DeltaError::Storage(e.to_string()))?;

    Ok(rows
        .into_iter()
        .map(|r| AuditEntry {
            id: r.id,
            user_id: r.user_id,
            action: r.action,
            resource_type: r.resource_type,
            resource_id: r.resource_id,
            details: r.details,
            ip_address: r.ip_address,
            created_at: r.created_at,
        })
        .collect())
}

#[derive(sqlx::FromRow)]
struct AuditRow {
    id: String,
    user_id: Option<String>,
    action: String,
    resource_type: String,
    resource_id: Option<String>,
    details: Option<String>,
    ip_address: Option<String>,
    created_at: String,
}
