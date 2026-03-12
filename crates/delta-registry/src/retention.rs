//! Artifact retention cleanup logic.

use delta_core::db;
use serde::Serialize;
use sqlx::SqlitePool;

use crate::BlobStore;

#[derive(Debug, Serialize)]
pub struct CleanupReport {
    pub repo_id: String,
    pub expired_deleted: usize,
    pub excess_deleted: usize,
    pub oversize_deleted: usize,
}

/// Run cleanup for a single repo using its retention policy (or global config fallback).
pub async fn cleanup_repo(
    pool: &SqlitePool,
    blob_store: &BlobStore,
    repo_id: &str,
    max_age_days: Option<i64>,
    max_count: Option<i64>,
    max_total_bytes: Option<i64>,
) -> Result<CleanupReport, delta_core::DeltaError> {
    let mut report = CleanupReport {
        repo_id: repo_id.to_string(),
        expired_deleted: 0,
        excess_deleted: 0,
        oversize_deleted: 0,
    };

    // Age-based cleanup
    if let Some(days) = max_age_days {
        let expired = db::retention::find_expired_artifacts(pool, repo_id, days).await?;
        for artifact in &expired {
            let _ = blob_store.delete(&artifact.content_hash);
            db::artifact::delete(pool, &artifact.id).await?;
        }
        report.expired_deleted = expired.len();
    }

    // Count-based cleanup
    if let Some(count) = max_count {
        let excess = db::retention::find_excess_artifacts(pool, repo_id, count).await?;
        for artifact in &excess {
            let _ = blob_store.delete(&artifact.content_hash);
            db::artifact::delete(pool, &artifact.id).await?;
        }
        report.excess_deleted = excess.len();
    }

    // Size-based cleanup
    if let Some(max_bytes) = max_total_bytes {
        let oversize = db::retention::find_oversize_artifacts(pool, repo_id, max_bytes).await?;
        for artifact in &oversize {
            let _ = blob_store.delete(&artifact.content_hash);
            db::artifact::delete(pool, &artifact.id).await?;
        }
        report.oversize_deleted = oversize.len();
    }

    Ok(report)
}
