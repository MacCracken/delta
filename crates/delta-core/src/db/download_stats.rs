use crate::{DeltaError, Result};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DailyCount {
    pub date: String,
    pub count: i64,
}

#[derive(sqlx::FromRow)]
struct DailyCountRow {
    date: String,
    count: i64,
}

pub async fn record_download(
    pool: &SqlitePool,
    artifact_id: &str,
    user_id: Option<&str>,
    user_agent: Option<&str>,
    ip_address: Option<&str>,
) -> Result<()> {
    let id = Uuid::new_v4().to_string();

    sqlx::query(
        "INSERT INTO download_events (id, artifact_id, user_id, user_agent, ip_address) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(artifact_id)
    .bind(user_id)
    .bind(user_agent)
    .bind(ip_address)
    .execute(pool)
    .await
    .map_err(|e| DeltaError::Registry(e.to_string()))?;

    // Also bump the counter on the artifact itself
    super::artifact::increment_download(pool, artifact_id).await?;
    Ok(())
}

/// Get daily download counts for the last N days.
pub async fn get_daily_counts(
    pool: &SqlitePool,
    artifact_id: &str,
    days: i64,
) -> Result<Vec<DailyCount>> {
    let cutoff = format!("-{} days", days);
    let rows = sqlx::query_as::<_, DailyCountRow>(
        "SELECT date(downloaded_at) as date, COUNT(*) as count
         FROM download_events
         WHERE artifact_id = ? AND downloaded_at >= datetime('now', ?)
         GROUP BY date(downloaded_at)
         ORDER BY date ASC",
    )
    .bind(artifact_id)
    .bind(&cutoff)
    .fetch_all(pool)
    .await
    .map_err(|e| DeltaError::Registry(e.to_string()))?;

    Ok(rows
        .into_iter()
        .map(|r| DailyCount {
            date: r.date,
            count: r.count,
        })
        .collect())
}

/// Get top downloaded artifacts for a repo.
pub async fn get_top_downloaded(
    pool: &SqlitePool,
    repo_id: &str,
    limit: i64,
) -> Result<Vec<TopArtifact>> {
    let rows = sqlx::query_as::<_, TopArtifactRow>(
        "SELECT id, name, download_count FROM artifacts WHERE repo_id = ? ORDER BY download_count DESC LIMIT ?",
    )
    .bind(repo_id)
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(|e| DeltaError::Registry(e.to_string()))?;

    Ok(rows
        .into_iter()
        .map(|r| TopArtifact {
            artifact_id: r.id,
            name: r.name,
            download_count: r.download_count,
        })
        .collect())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopArtifact {
    pub artifact_id: String,
    pub name: String,
    pub download_count: i64,
}

#[derive(sqlx::FromRow)]
struct TopArtifactRow {
    id: String,
    name: String,
    download_count: i64,
}
