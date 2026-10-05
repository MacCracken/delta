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
///
/// A limit of zero or less means "no limit": read literally it would delete
/// every artifact, which is never what a `0` in a config file means.
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
    let max_age_days = max_age_days.filter(|&days| days > 0);
    let max_count = max_count.filter(|&count| count > 0);
    let max_total_bytes = max_total_bytes.filter(|&bytes| bytes > 0);

    // Age-based cleanup
    if let Some(days) = max_age_days {
        let expired = db::retention::find_expired_artifacts(pool, repo_id, days).await?;
        for artifact in &expired {
            db::artifact::delete(pool, &artifact.id).await?;
            crate::store::release_blob(pool, blob_store, &artifact.content_hash).await?;
        }
        report.expired_deleted = expired.len();
    }

    // Count-based cleanup
    if let Some(count) = max_count {
        let excess = db::retention::find_excess_artifacts(pool, repo_id, count).await?;
        for artifact in &excess {
            db::artifact::delete(pool, &artifact.id).await?;
            crate::store::release_blob(pool, blob_store, &artifact.content_hash).await?;
        }
        report.excess_deleted = excess.len();
    }

    // Size-based cleanup
    if let Some(max_bytes) = max_total_bytes {
        let oversize = db::retention::find_oversize_artifacts(pool, repo_id, max_bytes).await?;
        for artifact in &oversize {
            db::artifact::delete(pool, &artifact.id).await?;
            crate::store::release_blob(pool, blob_store, &artifact.content_hash).await?;
        }
        report.oversize_deleted = oversize.len();
    }

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_zero_limits_mean_no_limit() {
        let tmp = tempfile::tempdir().unwrap();
        let url = format!("sqlite://{}", tmp.path().join("delta.db").display());
        let pool = db::init_pool(&url).await.unwrap();
        let store = BlobStore::new(tmp.path().join("blobs"));
        let user = db::user::create(&pool, "alice", "a@example.com", "pw", false)
            .await
            .unwrap();
        let repo = db::repo::create(
            &pool,
            &user.id.to_string(),
            "r",
            None,
            delta_core::models::repo::Visibility::Public,
        )
        .await
        .unwrap();
        let repo_id = repo.id.to_string();
        for (i, data) in [b"one".as_slice(), b"two".as_slice()]
            .into_iter()
            .enumerate()
        {
            let hash = store.store(data).unwrap();
            db::artifact::create(
                &pool,
                &db::artifact::CreateArtifactParams {
                    repo_id: &repo_id,
                    pipeline_id: None,
                    name: &format!("a{i}"),
                    version: None,
                    artifact_type: "generic",
                    content_hash: &hash,
                    size_bytes: 3,
                    metadata: None,
                },
            )
            .await
            .unwrap();
        }

        let report = cleanup_repo(&pool, &store, &repo_id, Some(0), Some(0), Some(0))
            .await
            .unwrap();
        assert_eq!(
            (
                report.expired_deleted,
                report.excess_deleted,
                report.oversize_deleted
            ),
            (0, 0, 0)
        );
        let report = cleanup_repo(&pool, &store, &repo_id, None, Some(1), None)
            .await
            .unwrap();
        assert_eq!(report.excess_deleted, 1);
    }
}
