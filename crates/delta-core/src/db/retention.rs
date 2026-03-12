use crate::{DeltaError, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetentionPolicy {
    pub id: String,
    pub repo_id: String,
    pub max_age_days: Option<i64>,
    pub max_count: Option<i64>,
    pub max_total_bytes: Option<i64>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(sqlx::FromRow)]
struct RetentionRow {
    id: String,
    repo_id: String,
    max_age_days: Option<i64>,
    max_count: Option<i64>,
    max_total_bytes: Option<i64>,
    created_at: String,
    updated_at: String,
}

impl RetentionRow {
    fn into_policy(self) -> RetentionPolicy {
        RetentionPolicy {
            id: self.id,
            repo_id: self.repo_id,
            max_age_days: self.max_age_days,
            max_count: self.max_count,
            max_total_bytes: self.max_total_bytes,
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

pub async fn get_policy(pool: &SqlitePool, repo_id: &str) -> Result<Option<RetentionPolicy>> {
    let row = sqlx::query_as::<_, RetentionRow>(
        "SELECT * FROM artifact_retention_policies WHERE repo_id = ?",
    )
    .bind(repo_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| DeltaError::Registry(e.to_string()))?;
    Ok(row.map(|r| r.into_policy()))
}

pub struct SetPolicyParams<'a> {
    pub repo_id: &'a str,
    pub max_age_days: Option<i64>,
    pub max_count: Option<i64>,
    pub max_total_bytes: Option<i64>,
}

pub async fn set_policy(
    pool: &SqlitePool,
    params: &SetPolicyParams<'_>,
) -> Result<RetentionPolicy> {
    let now = Utc::now().to_rfc3339();

    // Upsert: insert or update on conflict
    let id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO artifact_retention_policies (id, repo_id, max_age_days, max_count, max_total_bytes, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(repo_id) DO UPDATE SET
            max_age_days = excluded.max_age_days,
            max_count = excluded.max_count,
            max_total_bytes = excluded.max_total_bytes,
            updated_at = excluded.updated_at",
    )
    .bind(&id)
    .bind(params.repo_id)
    .bind(params.max_age_days)
    .bind(params.max_count)
    .bind(params.max_total_bytes)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(|e| DeltaError::Registry(e.to_string()))?;

    get_policy(pool, params.repo_id)
        .await?
        .ok_or_else(|| DeltaError::Registry("failed to read back policy".into()))
}

/// Find artifacts older than `max_age_days` in the given repo.
pub async fn find_expired_artifacts(
    pool: &SqlitePool,
    repo_id: &str,
    max_age_days: i64,
) -> Result<Vec<super::artifact::Artifact>> {
    let cutoff = format!("-{} days", max_age_days);
    let rows = sqlx::query_as::<_, super::artifact::ArtifactRowPub>(
        "SELECT * FROM artifacts WHERE repo_id = ? AND created_at < datetime('now', ?) ORDER BY created_at ASC",
    )
    .bind(repo_id)
    .bind(&cutoff)
    .fetch_all(pool)
    .await
    .map_err(|e| DeltaError::Registry(e.to_string()))?;
    Ok(rows.into_iter().map(|r| r.into_artifact()).collect())
}

/// Find the oldest artifacts that exceed `max_count` for a repo.
pub async fn find_excess_artifacts(
    pool: &SqlitePool,
    repo_id: &str,
    max_count: i64,
) -> Result<Vec<super::artifact::Artifact>> {
    let rows = sqlx::query_as::<_, super::artifact::ArtifactRowPub>(
        "SELECT * FROM artifacts WHERE repo_id = ? ORDER BY created_at DESC LIMIT -1 OFFSET ?",
    )
    .bind(repo_id)
    .bind(max_count)
    .fetch_all(pool)
    .await
    .map_err(|e| DeltaError::Registry(e.to_string()))?;
    Ok(rows.into_iter().map(|r| r.into_artifact()).collect())
}

/// Get total artifact size for a repo.
pub async fn total_artifact_size(pool: &SqlitePool, repo_id: &str) -> Result<i64> {
    let row: (i64,) =
        sqlx::query_as("SELECT COALESCE(SUM(size_bytes), 0) FROM artifacts WHERE repo_id = ?")
            .bind(repo_id)
            .fetch_one(pool)
            .await
            .map_err(|e| DeltaError::Registry(e.to_string()))?;
    Ok(row.0)
}

/// Find oldest artifacts to bring total size under `max_total_bytes`.
pub async fn find_oversize_artifacts(
    pool: &SqlitePool,
    repo_id: &str,
    max_total_bytes: i64,
) -> Result<Vec<super::artifact::Artifact>> {
    let total = total_artifact_size(pool, repo_id).await?;
    if total <= max_total_bytes {
        return Ok(vec![]);
    }

    // Get all artifacts oldest first, accumulate until we've freed enough
    let rows = sqlx::query_as::<_, super::artifact::ArtifactRowPub>(
        "SELECT * FROM artifacts WHERE repo_id = ? ORDER BY created_at ASC",
    )
    .bind(repo_id)
    .fetch_all(pool)
    .await
    .map_err(|e| DeltaError::Registry(e.to_string()))?;

    let mut to_delete = vec![];
    let mut freed: i64 = 0;
    let excess = total - max_total_bytes;
    for row in rows {
        if freed >= excess {
            break;
        }
        freed += row.size_bytes;
        to_delete.push(row.into_artifact());
    }
    Ok(to_delete)
}
