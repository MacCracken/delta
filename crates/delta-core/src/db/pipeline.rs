use crate::{DeltaError, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Queued,
    Running,
    Passed,
    Failed,
    Cancelled,
}

impl RunStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Passed => "passed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "running" => Self::Running,
            "passed" => Self::Passed,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            _ => Self::Queued,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineRun {
    pub id: String,
    pub repo_id: String,
    pub workflow_name: String,
    pub trigger_type: String,
    pub trigger_ref: Option<String>,
    pub commit_sha: String,
    pub status: RunStatus,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobRun {
    pub id: String,
    pub pipeline_id: String,
    pub job_name: String,
    pub status: RunStatus,
    pub runner: Option<String>,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub exit_code: Option<i32>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepLog {
    pub id: String,
    pub job_id: String,
    pub step_name: String,
    pub step_index: i64,
    pub output: String,
    pub status: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
}

pub async fn create_pipeline(
    pool: &SqlitePool,
    repo_id: &str,
    workflow_name: &str,
    trigger_type: &str,
    trigger_ref: Option<&str>,
    commit_sha: &str,
) -> Result<PipelineRun> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();

    sqlx::query(
        "INSERT INTO pipeline_runs (id, repo_id, workflow_name, trigger_type, trigger_ref, commit_sha, status, created_at)
         VALUES (?, ?, ?, ?, ?, ?, 'queued', ?)",
    )
    .bind(&id)
    .bind(repo_id)
    .bind(workflow_name)
    .bind(trigger_type)
    .bind(trigger_ref)
    .bind(commit_sha)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(|e| DeltaError::Pipeline(e.to_string()))?;

    get_pipeline(pool, &id).await
}

pub async fn get_pipeline(pool: &SqlitePool, id: &str) -> Result<PipelineRun> {
    let row = sqlx::query_as::<_, PipelineRow>("SELECT * FROM pipeline_runs WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|e| DeltaError::Pipeline(e.to_string()))?
        .ok_or_else(|| DeltaError::Pipeline("pipeline not found".into()))?;
    Ok(row.into_run())
}

pub async fn list_pipelines(
    pool: &SqlitePool,
    repo_id: &str,
    status: Option<&str>,
    limit: i64,
) -> Result<Vec<PipelineRun>> {
    let rows = if let Some(s) = status {
        sqlx::query_as::<_, PipelineRow>(
            "SELECT * FROM pipeline_runs WHERE repo_id = ? AND status = ? ORDER BY created_at DESC LIMIT ?",
        )
        .bind(repo_id)
        .bind(s)
        .bind(limit)
        .fetch_all(pool)
        .await
    } else {
        sqlx::query_as::<_, PipelineRow>(
            "SELECT * FROM pipeline_runs WHERE repo_id = ? ORDER BY created_at DESC LIMIT ?",
        )
        .bind(repo_id)
        .bind(limit)
        .fetch_all(pool)
        .await
    }
    .map_err(|e| DeltaError::Pipeline(e.to_string()))?;

    Ok(rows.into_iter().map(|r| r.into_run()).collect())
}

pub async fn update_pipeline_status(
    pool: &SqlitePool,
    id: &str,
    status: RunStatus,
) -> Result<PipelineRun> {
    let now = Utc::now().to_rfc3339();
    match status {
        RunStatus::Running => {
            sqlx::query("UPDATE pipeline_runs SET status = ?, started_at = ? WHERE id = ?")
                .bind(status.as_str())
                .bind(&now)
                .bind(id)
                .execute(pool)
                .await
        }
        RunStatus::Passed | RunStatus::Failed | RunStatus::Cancelled => {
            sqlx::query("UPDATE pipeline_runs SET status = ?, finished_at = ? WHERE id = ?")
                .bind(status.as_str())
                .bind(&now)
                .bind(id)
                .execute(pool)
                .await
        }
        _ => {
            sqlx::query("UPDATE pipeline_runs SET status = ? WHERE id = ?")
                .bind(status.as_str())
                .bind(id)
                .execute(pool)
                .await
        }
    }
    .map_err(|e| DeltaError::Pipeline(e.to_string()))?;

    get_pipeline(pool, id).await
}

pub async fn create_job(pool: &SqlitePool, pipeline_id: &str, job_name: &str) -> Result<JobRun> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();

    sqlx::query(
        "INSERT INTO job_runs (id, pipeline_id, job_name, status, created_at) VALUES (?, ?, ?, 'queued', ?)",
    )
    .bind(&id)
    .bind(pipeline_id)
    .bind(job_name)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(|e| DeltaError::Pipeline(e.to_string()))?;

    get_job(pool, &id).await
}

pub async fn get_job(pool: &SqlitePool, id: &str) -> Result<JobRun> {
    let row = sqlx::query_as::<_, JobRow>("SELECT * FROM job_runs WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|e| DeltaError::Pipeline(e.to_string()))?
        .ok_or_else(|| DeltaError::Pipeline("job not found".into()))?;
    Ok(row.into_job())
}

pub async fn list_jobs(pool: &SqlitePool, pipeline_id: &str) -> Result<Vec<JobRun>> {
    let rows = sqlx::query_as::<_, JobRow>(
        "SELECT * FROM job_runs WHERE pipeline_id = ? ORDER BY created_at",
    )
    .bind(pipeline_id)
    .fetch_all(pool)
    .await
    .map_err(|e| DeltaError::Pipeline(e.to_string()))?;
    Ok(rows.into_iter().map(|r| r.into_job()).collect())
}

pub async fn update_job_status(
    pool: &SqlitePool,
    id: &str,
    status: RunStatus,
    exit_code: Option<i32>,
) -> Result<JobRun> {
    let now = Utc::now().to_rfc3339();
    match status {
        RunStatus::Running => {
            sqlx::query("UPDATE job_runs SET status = ?, started_at = ? WHERE id = ?")
                .bind(status.as_str())
                .bind(&now)
                .bind(id)
                .execute(pool)
                .await
        }
        RunStatus::Passed | RunStatus::Failed | RunStatus::Cancelled => {
            sqlx::query(
                "UPDATE job_runs SET status = ?, finished_at = ?, exit_code = ? WHERE id = ?",
            )
            .bind(status.as_str())
            .bind(&now)
            .bind(exit_code)
            .bind(id)
            .execute(pool)
            .await
        }
        _ => {
            sqlx::query("UPDATE job_runs SET status = ? WHERE id = ?")
                .bind(status.as_str())
                .bind(id)
                .execute(pool)
                .await
        }
    }
    .map_err(|e| DeltaError::Pipeline(e.to_string()))?;

    get_job(pool, id).await
}

pub async fn append_step_log(
    pool: &SqlitePool,
    job_id: &str,
    step_name: &str,
    step_index: i64,
    output: &str,
    status: &str,
) -> Result<()> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();

    sqlx::query(
        "INSERT INTO step_logs (id, job_id, step_name, step_index, output, status, started_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(job_id)
    .bind(step_name)
    .bind(step_index)
    .bind(output)
    .bind(status)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(|e| DeltaError::Pipeline(e.to_string()))?;

    Ok(())
}

pub async fn get_step_logs(pool: &SqlitePool, job_id: &str) -> Result<Vec<StepLog>> {
    let rows = sqlx::query_as::<_, StepLogRow>(
        "SELECT * FROM step_logs WHERE job_id = ? ORDER BY step_index",
    )
    .bind(job_id)
    .fetch_all(pool)
    .await
    .map_err(|e| DeltaError::Pipeline(e.to_string()))?;

    Ok(rows
        .into_iter()
        .map(|r| StepLog {
            id: r.id,
            job_id: r.job_id,
            step_name: r.step_name,
            step_index: r.step_index,
            output: r.output,
            status: r.status,
            started_at: r.started_at,
            finished_at: r.finished_at,
        })
        .collect())
}

/// Get the latest pipeline run for a repo, optionally filtered by branch.
pub async fn get_latest(
    pool: &SqlitePool,
    repo_id: &str,
    branch: Option<&str>,
) -> Result<Option<PipelineRun>> {
    let row = if let Some(branch) = branch {
        sqlx::query_as::<_, PipelineRow>(
            "SELECT * FROM pipeline_runs WHERE repo_id = ? AND trigger_ref = ?
             ORDER BY created_at DESC LIMIT 1",
        )
        .bind(repo_id)
        .bind(branch)
        .fetch_optional(pool)
        .await
    } else {
        sqlx::query_as::<_, PipelineRow>(
            "SELECT * FROM pipeline_runs WHERE repo_id = ?
             ORDER BY created_at DESC LIMIT 1",
        )
        .bind(repo_id)
        .fetch_optional(pool)
        .await
    }
    .map_err(|e| DeltaError::Pipeline(e.to_string()))?;

    Ok(row.map(|r| r.into_run()))
}

// --- Row types ---

#[derive(sqlx::FromRow)]
struct PipelineRow {
    id: String,
    repo_id: String,
    workflow_name: String,
    trigger_type: String,
    trigger_ref: Option<String>,
    commit_sha: String,
    status: String,
    started_at: Option<String>,
    finished_at: Option<String>,
    created_at: String,
}

impl PipelineRow {
    fn into_run(self) -> PipelineRun {
        PipelineRun {
            id: self.id,
            repo_id: self.repo_id,
            workflow_name: self.workflow_name,
            trigger_type: self.trigger_type,
            trigger_ref: self.trigger_ref,
            commit_sha: self.commit_sha,
            status: RunStatus::parse(&self.status),
            started_at: self.started_at,
            finished_at: self.finished_at,
            created_at: self.created_at,
        }
    }
}

#[derive(sqlx::FromRow)]
struct JobRow {
    id: String,
    pipeline_id: String,
    job_name: String,
    status: String,
    runner: Option<String>,
    started_at: Option<String>,
    finished_at: Option<String>,
    exit_code: Option<i32>,
    created_at: String,
}

impl JobRow {
    fn into_job(self) -> JobRun {
        JobRun {
            id: self.id,
            pipeline_id: self.pipeline_id,
            job_name: self.job_name,
            status: RunStatus::parse(&self.status),
            runner: self.runner,
            started_at: self.started_at,
            finished_at: self.finished_at,
            exit_code: self.exit_code,
            created_at: self.created_at,
        }
    }
}

#[derive(sqlx::FromRow)]
struct StepLogRow {
    id: String,
    job_id: String,
    step_name: String,
    step_index: i64,
    output: String,
    status: String,
    started_at: Option<String>,
    finished_at: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_run_status_roundtrip() {
        for status in [
            RunStatus::Queued,
            RunStatus::Running,
            RunStatus::Passed,
            RunStatus::Failed,
            RunStatus::Cancelled,
        ] {
            assert_eq!(RunStatus::parse(status.as_str()), status);
        }
    }

    #[test]
    fn test_run_status_parse_unknown() {
        assert_eq!(RunStatus::parse("unknown"), RunStatus::Queued);
        assert_eq!(RunStatus::parse(""), RunStatus::Queued);
    }

    #[test]
    fn test_run_status_as_str() {
        assert_eq!(RunStatus::Queued.as_str(), "queued");
        assert_eq!(RunStatus::Running.as_str(), "running");
        assert_eq!(RunStatus::Passed.as_str(), "passed");
        assert_eq!(RunStatus::Failed.as_str(), "failed");
        assert_eq!(RunStatus::Cancelled.as_str(), "cancelled");
    }
}
