use crate::models::pull_request::{CheckState, StatusCheck};
use crate::{DeltaError, Result};
use chrono::Utc;
use sqlx::SqlitePool;
use uuid::Uuid;

/// Create or update a status check for a commit.
pub async fn upsert(
    pool: &SqlitePool,
    repo_id: &str,
    commit_sha: &str,
    context: &str,
    state: CheckState,
    description: Option<&str>,
    target_url: Option<&str>,
) -> Result<StatusCheck> {
    let now = Utc::now().to_rfc3339();
    let state_str = serde_json::to_value(state)
        .unwrap()
        .as_str()
        .unwrap()
        .to_string();

    // Try update first
    let result = sqlx::query(
        "UPDATE status_checks SET state = ?, description = ?, target_url = ?, updated_at = ?
         WHERE repo_id = ? AND commit_sha = ? AND context = ?",
    )
    .bind(&state_str)
    .bind(description)
    .bind(target_url)
    .bind(&now)
    .bind(repo_id)
    .bind(commit_sha)
    .bind(context)
    .execute(pool)
    .await
    .map_err(|e| DeltaError::Storage(e.to_string()))?;

    if result.rows_affected() == 0 {
        let id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO status_checks (id, repo_id, commit_sha, context, state, description, target_url, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(repo_id)
        .bind(commit_sha)
        .bind(context)
        .bind(&state_str)
        .bind(description)
        .bind(target_url)
        .bind(&now)
        .bind(&now)
        .execute(pool)
        .await
        .map_err(|e| DeltaError::Storage(e.to_string()))?;
    }

    get_for_commit(pool, repo_id, commit_sha)
        .await?
        .into_iter()
        .find(|c| c.context == context)
        .ok_or_else(|| DeltaError::Storage("failed to retrieve status check".into()))
}

/// Get all status checks for a commit.
pub async fn get_for_commit(
    pool: &SqlitePool,
    repo_id: &str,
    commit_sha: &str,
) -> Result<Vec<StatusCheck>> {
    let rows = sqlx::query_as::<_, CheckRow>(
        "SELECT * FROM status_checks WHERE repo_id = ? AND commit_sha = ? ORDER BY context",
    )
    .bind(repo_id)
    .bind(commit_sha)
    .fetch_all(pool)
    .await
    .map_err(|e| DeltaError::Storage(e.to_string()))?;
    Ok(rows.into_iter().map(|r| r.into_check()).collect())
}

/// Check if all status checks for a commit have passed.
pub async fn all_passed(pool: &SqlitePool, repo_id: &str, commit_sha: &str) -> Result<bool> {
    let checks = get_for_commit(pool, repo_id, commit_sha).await?;
    if checks.is_empty() {
        return Ok(true); // No checks = nothing to fail
    }
    Ok(checks.iter().all(|c| c.state == CheckState::Success))
}

#[derive(sqlx::FromRow)]
struct CheckRow {
    id: String,
    repo_id: String,
    commit_sha: String,
    context: String,
    state: String,
    description: Option<String>,
    target_url: Option<String>,
    created_at: String,
    updated_at: String,
}

impl CheckRow {
    fn into_check(self) -> StatusCheck {
        StatusCheck {
            id: self.id.parse().unwrap_or_default(),
            repo_id: self.repo_id.parse().unwrap_or_default(),
            commit_sha: self.commit_sha,
            context: self.context,
            state: match self.state.as_str() {
                "success" => CheckState::Success,
                "failure" => CheckState::Failure,
                "error" => CheckState::Error,
                _ => CheckState::Pending,
            },
            description: self.description,
            target_url: self.target_url,
            created_at: self.created_at.parse().unwrap_or_default(),
            updated_at: self.updated_at.parse().unwrap_or_default(),
        }
    }
}
