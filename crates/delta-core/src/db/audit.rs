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

impl AuditRow {
    fn into_entry(self) -> AuditEntry {
        AuditEntry {
            id: self.id,
            user_id: self.user_id,
            action: self.action,
            resource_type: self.resource_type,
            resource_id: self.resource_id,
            details: self.details,
            ip_address: self.ip_address,
            created_at: self.created_at,
        }
    }
}

/// List audit entries filtered by date range for compliance export.
pub async fn list_for_export(
    pool: &SqlitePool,
    since: Option<&str>,
    until: Option<&str>,
    resource_type: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<Vec<AuditEntry>> {
    let mut sql = String::from("SELECT * FROM audit_log WHERE 1=1");
    let mut binds: Vec<String> = Vec::new();

    if let Some(s) = since {
        sql.push_str(" AND created_at >= ?");
        binds.push(s.to_string());
    }
    if let Some(u) = until {
        sql.push_str(" AND created_at <= ?");
        binds.push(u.to_string());
    }
    if let Some(rt) = resource_type {
        sql.push_str(" AND resource_type = ?");
        binds.push(rt.to_string());
    }
    sql.push_str(" ORDER BY created_at ASC LIMIT ? OFFSET ?");

    let mut query = sqlx::query_as::<_, AuditRow>(&sql);
    for b in &binds {
        query = query.bind(b);
    }
    query = query.bind(limit).bind(offset);

    let rows = query
        .fetch_all(pool)
        .await
        .map_err(|e| DeltaError::Storage(e.to_string()))?;

    Ok(rows.into_iter().map(|r| r.into_entry()).collect())
}

/// Count audit entries matching export filters (for pagination).
pub async fn count_for_export(
    pool: &SqlitePool,
    since: Option<&str>,
    until: Option<&str>,
    resource_type: Option<&str>,
) -> Result<i64> {
    let mut sql = String::from("SELECT COUNT(*) as count FROM audit_log WHERE 1=1");
    let mut binds: Vec<String> = Vec::new();

    if let Some(s) = since {
        sql.push_str(" AND created_at >= ?");
        binds.push(s.to_string());
    }
    if let Some(u) = until {
        sql.push_str(" AND created_at <= ?");
        binds.push(u.to_string());
    }
    if let Some(rt) = resource_type {
        sql.push_str(" AND resource_type = ?");
        binds.push(rt.to_string());
    }

    let mut query = sqlx::query_scalar::<_, i64>(&sql);
    for b in &binds {
        query = query.bind(b);
    }

    query
        .fetch_one(pool)
        .await
        .map_err(|e| DeltaError::Storage(e.to_string()))
}
