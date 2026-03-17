use crate::{DeltaError, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Runner {
    pub id: String,
    pub name: String,
    pub labels: Vec<String>,
    pub status: RunnerStatus,
    pub last_heartbeat_at: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunnerStatus {
    Online,
    Offline,
}

impl RunnerStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Online => "online",
            Self::Offline => "offline",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "online" => Self::Online,
            _ => Self::Offline,
        }
    }
}

/// A queued job waiting for a remote runner to pick it up.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueuedJob {
    pub id: String,
    pub job_run_id: String,
    pub pipeline_id: String,
    pub repo_id: String,
    pub labels: Vec<String>,
    pub payload: String,
    pub status: String,
    pub claimed_by: Option<String>,
    pub created_at: String,
}

/// Register a new runner. Returns the runner record.
pub async fn register(
    pool: &SqlitePool,
    name: &str,
    token_hash: &str,
    labels: &[String],
) -> Result<Runner> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    let labels_str = labels.join(",");

    sqlx::query(
        "INSERT INTO runners (id, name, token_hash, labels, status, last_heartbeat_at, created_at)
         VALUES (?, ?, ?, ?, 'online', ?, ?)
         ON CONFLICT(name) DO UPDATE SET
           token_hash = excluded.token_hash,
           labels = excluded.labels,
           status = 'online',
           last_heartbeat_at = excluded.last_heartbeat_at",
    )
    .bind(&id)
    .bind(name)
    .bind(token_hash)
    .bind(&labels_str)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(|e| DeltaError::Pipeline(e.to_string()))?;

    get_by_name(pool, name).await
}

pub async fn get(pool: &SqlitePool, id: &str) -> Result<Runner> {
    let row = sqlx::query_as::<_, RunnerRow>("SELECT * FROM runners WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|e| DeltaError::Pipeline(e.to_string()))?
        .ok_or_else(|| DeltaError::Pipeline("runner not found".into()))?;
    Ok(row.into_runner())
}

pub async fn get_by_name(pool: &SqlitePool, name: &str) -> Result<Runner> {
    let row = sqlx::query_as::<_, RunnerRow>("SELECT * FROM runners WHERE name = ?")
        .bind(name)
        .fetch_optional(pool)
        .await
        .map_err(|e| DeltaError::Pipeline(e.to_string()))?
        .ok_or_else(|| DeltaError::Pipeline("runner not found".into()))?;
    Ok(row.into_runner())
}

pub async fn list(pool: &SqlitePool) -> Result<Vec<Runner>> {
    let rows = sqlx::query_as::<_, RunnerRow>("SELECT * FROM runners ORDER BY name")
        .fetch_all(pool)
        .await
        .map_err(|e| DeltaError::Pipeline(e.to_string()))?;
    Ok(rows.into_iter().map(|r| r.into_runner()).collect())
}

pub async fn delete(pool: &SqlitePool, id: &str) -> Result<()> {
    let result = sqlx::query("DELETE FROM runners WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await
        .map_err(|e| DeltaError::Pipeline(e.to_string()))?;

    if result.rows_affected() == 0 {
        return Err(DeltaError::Pipeline("runner not found".into()));
    }
    Ok(())
}

pub async fn heartbeat(pool: &SqlitePool, runner_id: &str) -> Result<()> {
    let now = Utc::now().to_rfc3339();
    sqlx::query("UPDATE runners SET status = 'online', last_heartbeat_at = ? WHERE id = ?")
        .bind(&now)
        .bind(runner_id)
        .execute(pool)
        .await
        .map_err(|e| DeltaError::Pipeline(e.to_string()))?;
    Ok(())
}

/// Authenticate a runner by name and token hash. Returns the runner if valid.
pub async fn authenticate(pool: &SqlitePool, name: &str, token_hash: &str) -> Result<Runner> {
    let row = sqlx::query_as::<_, RunnerRow>(
        "SELECT * FROM runners WHERE name = ? AND token_hash = ?",
    )
    .bind(name)
    .bind(token_hash)
    .fetch_optional(pool)
    .await
    .map_err(|e| DeltaError::Pipeline(e.to_string()))?
    .ok_or_else(|| DeltaError::Pipeline("invalid runner credentials".into()))?;
    Ok(row.into_runner())
}

/// Enqueue a job for remote execution by a self-hosted runner.
pub async fn enqueue_job(
    pool: &SqlitePool,
    job_run_id: &str,
    pipeline_id: &str,
    repo_id: &str,
    labels: &[String],
    payload: &str,
) -> Result<QueuedJob> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    let labels_str = labels.join(",");

    sqlx::query(
        "INSERT INTO runner_job_queue (id, job_run_id, pipeline_id, repo_id, labels, payload, status, created_at)
         VALUES (?, ?, ?, ?, ?, ?, 'pending', ?)",
    )
    .bind(&id)
    .bind(job_run_id)
    .bind(pipeline_id)
    .bind(repo_id)
    .bind(&labels_str)
    .bind(payload)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(|e| DeltaError::Pipeline(e.to_string()))?;

    get_queued_job(pool, &id).await
}

/// Poll for the next pending job matching the runner's labels.
/// Claims the job atomically so no other runner can take it.
pub async fn poll_job(
    pool: &SqlitePool,
    runner_id: &str,
    runner_labels: &[String],
) -> Result<Option<QueuedJob>> {
    // Find a pending job whose required labels are a subset of the runner's labels.
    // Jobs with empty labels match any runner.
    let rows = sqlx::query_as::<_, QueuedJobRow>(
        "SELECT * FROM runner_job_queue WHERE status = 'pending' ORDER BY created_at ASC LIMIT 20",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| DeltaError::Pipeline(e.to_string()))?;

    for row in rows {
        let job_labels: Vec<String> = row
            .labels
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        // Check if runner has all required labels
        let matches = job_labels
            .iter()
            .all(|l| runner_labels.iter().any(|rl| rl == l));

        if matches {
            // Claim the job atomically
            let result = sqlx::query(
                "UPDATE runner_job_queue SET status = 'claimed', claimed_by = ? WHERE id = ? AND status = 'pending'",
            )
            .bind(runner_id)
            .bind(&row.id)
            .execute(pool)
            .await
            .map_err(|e| DeltaError::Pipeline(e.to_string()))?;

            if result.rows_affected() > 0 {
                return Ok(Some(get_queued_job(pool, &row.id).await?));
            }
        }
    }

    Ok(None)
}

pub async fn get_queued_job(pool: &SqlitePool, id: &str) -> Result<QueuedJob> {
    let row = sqlx::query_as::<_, QueuedJobRow>(
        "SELECT * FROM runner_job_queue WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(|e| DeltaError::Pipeline(e.to_string()))?
    .ok_or_else(|| DeltaError::Pipeline("queued job not found".into()))?;
    Ok(row.into_queued_job())
}

/// Mark a queued job as complete and remove it from the queue.
pub async fn complete_queued_job(pool: &SqlitePool, queue_id: &str) -> Result<()> {
    sqlx::query("UPDATE runner_job_queue SET status = 'completed' WHERE id = ?")
        .bind(queue_id)
        .execute(pool)
        .await
        .map_err(|e| DeltaError::Pipeline(e.to_string()))?;
    Ok(())
}

// --- Row types ---

#[derive(sqlx::FromRow)]
struct RunnerRow {
    id: String,
    name: String,
    #[allow(dead_code)]
    token_hash: String,
    labels: String,
    status: String,
    last_heartbeat_at: Option<String>,
    created_at: String,
}

impl RunnerRow {
    fn into_runner(self) -> Runner {
        Runner {
            id: self.id,
            name: self.name,
            labels: self
                .labels
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect(),
            status: RunnerStatus::parse(&self.status),
            last_heartbeat_at: self.last_heartbeat_at,
            created_at: self.created_at,
        }
    }
}

#[derive(sqlx::FromRow)]
struct QueuedJobRow {
    id: String,
    job_run_id: String,
    pipeline_id: String,
    repo_id: String,
    labels: String,
    payload: String,
    status: String,
    claimed_by: Option<String>,
    created_at: String,
}

impl QueuedJobRow {
    fn into_queued_job(self) -> QueuedJob {
        QueuedJob {
            id: self.id,
            job_run_id: self.job_run_id,
            pipeline_id: self.pipeline_id,
            repo_id: self.repo_id,
            labels: self
                .labels
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect(),
            payload: self.payload,
            status: self.status,
            claimed_by: self.claimed_by,
            created_at: self.created_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_runner_status_roundtrip() {
        assert_eq!(RunnerStatus::parse("online"), RunnerStatus::Online);
        assert_eq!(RunnerStatus::parse("offline"), RunnerStatus::Offline);
        assert_eq!(RunnerStatus::parse("unknown"), RunnerStatus::Offline);
    }

    #[test]
    fn test_runner_status_as_str() {
        assert_eq!(RunnerStatus::Online.as_str(), "online");
        assert_eq!(RunnerStatus::Offline.as_str(), "offline");
    }
}
