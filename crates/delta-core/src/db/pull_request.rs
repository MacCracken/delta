use crate::models::pull_request::*;
use crate::{DeltaError, Result};
use chrono::Utc;
use sqlx::SqlitePool;
use uuid::Uuid;

// --- Pull Requests ---

/// Allocate the next PR number for a repo (atomic via transaction).
async fn next_pr_number(pool: &SqlitePool, repo_id: &str) -> Result<i64> {
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| DeltaError::Storage(e.to_string()))?;

    sqlx::query(
        "INSERT INTO pr_counters (repo_id, next_number) VALUES (?, 1)
         ON CONFLICT(repo_id) DO UPDATE SET next_number = next_number + 1",
    )
    .bind(repo_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| DeltaError::Storage(e.to_string()))?;

    let row: (i64,) =
        sqlx::query_as("SELECT next_number FROM pr_counters WHERE repo_id = ?")
            .bind(repo_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| DeltaError::Storage(e.to_string()))?;

    tx.commit()
        .await
        .map_err(|e| DeltaError::Storage(e.to_string()))?;

    Ok(row.0)
}

pub struct CreatePrParams<'a> {
    pub repo_id: &'a str,
    pub author_id: &'a str,
    pub title: &'a str,
    pub body: Option<&'a str>,
    pub head_branch: &'a str,
    pub base_branch: &'a str,
    pub head_sha: Option<&'a str>,
    pub is_draft: bool,
}

pub async fn create(pool: &SqlitePool, params: CreatePrParams<'_>) -> Result<PullRequest> {
    let id = Uuid::new_v4().to_string();
    let number = next_pr_number(pool, params.repo_id).await?;
    let now = Utc::now().to_rfc3339();

    sqlx::query(
        "INSERT INTO pull_requests (id, number, repo_id, author_id, title, body, state, head_branch, base_branch, head_sha, is_draft, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, 'open', ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(number)
    .bind(params.repo_id)
    .bind(params.author_id)
    .bind(params.title)
    .bind(params.body)
    .bind(params.head_branch)
    .bind(params.base_branch)
    .bind(params.head_sha)
    .bind(params.is_draft)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(|e| DeltaError::Storage(e.to_string()))?;

    get_by_id(pool, &id).await
}

pub async fn get_by_id(pool: &SqlitePool, id: &str) -> Result<PullRequest> {
    let row = sqlx::query_as::<_, PrRow>("SELECT * FROM pull_requests WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|e| DeltaError::Storage(e.to_string()))?
        .ok_or_else(|| DeltaError::RepoNotFound("pull request not found".into()))?;
    Ok(row.into_pr())
}

pub async fn get_by_number(pool: &SqlitePool, repo_id: &str, number: i64) -> Result<PullRequest> {
    let row = sqlx::query_as::<_, PrRow>(
        "SELECT * FROM pull_requests WHERE repo_id = ? AND number = ?",
    )
    .bind(repo_id)
    .bind(number)
    .fetch_optional(pool)
    .await
    .map_err(|e| DeltaError::Storage(e.to_string()))?
    .ok_or_else(|| DeltaError::RepoNotFound(format!("pull request #{} not found", number)))?;
    Ok(row.into_pr())
}

pub async fn list_for_repo(
    pool: &SqlitePool,
    repo_id: &str,
    state_filter: Option<&str>,
) -> Result<Vec<PullRequest>> {
    let rows = if let Some(state) = state_filter {
        sqlx::query_as::<_, PrRow>(
            "SELECT * FROM pull_requests WHERE repo_id = ? AND state = ? ORDER BY number DESC LIMIT 100",
        )
        .bind(repo_id)
        .bind(state)
        .fetch_all(pool)
        .await
    } else {
        sqlx::query_as::<_, PrRow>(
            "SELECT * FROM pull_requests WHERE repo_id = ? ORDER BY number DESC LIMIT 100",
        )
        .bind(repo_id)
        .fetch_all(pool)
        .await
    }
    .map_err(|e| DeltaError::Storage(e.to_string()))?;

    Ok(rows.into_iter().map(|r| r.into_pr()).collect())
}

pub async fn update_title_body(
    pool: &SqlitePool,
    id: &str,
    title: Option<&str>,
    body: Option<&str>,
) -> Result<PullRequest> {
    let now = Utc::now().to_rfc3339();
    if let Some(t) = title {
        sqlx::query("UPDATE pull_requests SET title = ?, updated_at = ? WHERE id = ?")
            .bind(t)
            .bind(&now)
            .bind(id)
            .execute(pool)
            .await
            .map_err(|e| DeltaError::Storage(e.to_string()))?;
    }
    if let Some(b) = body {
        sqlx::query("UPDATE pull_requests SET body = ?, updated_at = ? WHERE id = ?")
            .bind(b)
            .bind(&now)
            .bind(id)
            .execute(pool)
            .await
            .map_err(|e| DeltaError::Storage(e.to_string()))?;
    }
    get_by_id(pool, id).await
}

pub async fn close(pool: &SqlitePool, id: &str) -> Result<PullRequest> {
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "UPDATE pull_requests SET state = 'closed', closed_at = ?, updated_at = ? WHERE id = ? AND state = 'open'",
    )
    .bind(&now)
    .bind(&now)
    .bind(id)
    .execute(pool)
    .await
    .map_err(|e| DeltaError::Storage(e.to_string()))?;
    get_by_id(pool, id).await
}

pub async fn reopen(pool: &SqlitePool, id: &str) -> Result<PullRequest> {
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "UPDATE pull_requests SET state = 'open', closed_at = NULL, updated_at = ? WHERE id = ? AND state = 'closed'",
    )
    .bind(&now)
    .bind(id)
    .execute(pool)
    .await
    .map_err(|e| DeltaError::Storage(e.to_string()))?;
    get_by_id(pool, id).await
}

pub async fn mark_merged(
    pool: &SqlitePool,
    id: &str,
    merged_by: &str,
    strategy: &str,
) -> Result<PullRequest> {
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "UPDATE pull_requests SET state = 'merged', merged_by = ?, merge_strategy = ?, merged_at = ?, updated_at = ? WHERE id = ? AND state = 'open'",
    )
    .bind(merged_by)
    .bind(strategy)
    .bind(&now)
    .bind(&now)
    .bind(id)
    .execute(pool)
    .await
    .map_err(|e| DeltaError::Storage(e.to_string()))?;
    get_by_id(pool, id).await
}

// --- Comments ---

pub async fn add_comment(
    pool: &SqlitePool,
    pr_id: &str,
    author_id: &str,
    body: &str,
    file_path: Option<&str>,
    line: Option<i64>,
    side: Option<&str>,
) -> Result<PrComment> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();

    sqlx::query(
        "INSERT INTO pr_comments (id, pr_id, author_id, body, file_path, line, side, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(pr_id)
    .bind(author_id)
    .bind(body)
    .bind(file_path)
    .bind(line)
    .bind(side)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(|e| DeltaError::Storage(e.to_string()))?;

    get_comment_by_id(pool, &id).await
}

pub async fn get_comment_by_id(pool: &SqlitePool, id: &str) -> Result<PrComment> {
    let row = sqlx::query_as::<_, CommentRow>("SELECT * FROM pr_comments WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|e| DeltaError::Storage(e.to_string()))?
        .ok_or_else(|| DeltaError::RepoNotFound("comment not found".into()))?;
    Ok(row.into_comment())
}

pub async fn list_comments(pool: &SqlitePool, pr_id: &str) -> Result<Vec<PrComment>> {
    let rows = sqlx::query_as::<_, CommentRow>(
        "SELECT * FROM pr_comments WHERE pr_id = ? ORDER BY created_at ASC",
    )
    .bind(pr_id)
    .fetch_all(pool)
    .await
    .map_err(|e| DeltaError::Storage(e.to_string()))?;
    Ok(rows.into_iter().map(|r| r.into_comment()).collect())
}

pub async fn update_comment(pool: &SqlitePool, id: &str, body: &str) -> Result<PrComment> {
    let now = Utc::now().to_rfc3339();
    sqlx::query("UPDATE pr_comments SET body = ?, updated_at = ? WHERE id = ?")
        .bind(body)
        .bind(&now)
        .bind(id)
        .execute(pool)
        .await
        .map_err(|e| DeltaError::Storage(e.to_string()))?;
    get_comment_by_id(pool, id).await
}

pub async fn delete_comment(pool: &SqlitePool, id: &str) -> Result<()> {
    let result = sqlx::query("DELETE FROM pr_comments WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await
        .map_err(|e| DeltaError::Storage(e.to_string()))?;
    if result.rows_affected() == 0 {
        return Err(DeltaError::RepoNotFound("comment not found".into()));
    }
    Ok(())
}

// --- Reviews ---

pub async fn submit_review(
    pool: &SqlitePool,
    pr_id: &str,
    reviewer_id: &str,
    state: ReviewState,
    body: Option<&str>,
) -> Result<PrReview> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    let state_str = state.as_str().to_string();

    sqlx::query(
        "INSERT INTO pr_reviews (id, pr_id, reviewer_id, state, body, created_at)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(pr_id)
    .bind(reviewer_id)
    .bind(&state_str)
    .bind(body)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(|e| DeltaError::Storage(e.to_string()))?;

    Ok(PrReview {
        id: id.parse().unwrap_or_default(),
        pr_id: pr_id.parse().unwrap_or_default(),
        reviewer_id: reviewer_id.parse().unwrap_or_default(),
        state,
        body: body.map(|b| b.to_string()),
        created_at: now.parse().unwrap_or_default(),
    })
}

pub async fn list_reviews(pool: &SqlitePool, pr_id: &str) -> Result<Vec<PrReview>> {
    let rows = sqlx::query_as::<_, ReviewRow>(
        "SELECT * FROM pr_reviews WHERE pr_id = ? ORDER BY created_at ASC",
    )
    .bind(pr_id)
    .fetch_all(pool)
    .await
    .map_err(|e| DeltaError::Storage(e.to_string()))?;
    Ok(rows.into_iter().map(|r| r.into_review()).collect())
}

/// Count approvals for a PR (only the latest review per reviewer counts).
pub async fn count_approvals(pool: &SqlitePool, pr_id: &str) -> Result<u32> {
    let row: (i64,) = sqlx::query_as(
        "SELECT COUNT(DISTINCT reviewer_id) FROM pr_reviews
         WHERE pr_id = ? AND state = 'approved'
         AND id IN (
             SELECT id FROM pr_reviews r2
             WHERE r2.pr_id = pr_reviews.pr_id AND r2.reviewer_id = pr_reviews.reviewer_id
             ORDER BY created_at DESC LIMIT 1
         )",
    )
    .bind(pr_id)
    .fetch_one(pool)
    .await
    .map_err(|e| DeltaError::Storage(e.to_string()))?;
    Ok(row.0 as u32)
}

// --- Row types ---

#[derive(sqlx::FromRow)]
struct PrRow {
    id: String,
    number: i64,
    repo_id: String,
    author_id: String,
    title: String,
    body: Option<String>,
    state: String,
    head_branch: String,
    base_branch: String,
    head_sha: Option<String>,
    is_draft: bool,
    merged_by: Option<String>,
    merge_strategy: Option<String>,
    created_at: String,
    updated_at: String,
    merged_at: Option<String>,
    closed_at: Option<String>,
}

impl PrRow {
    fn into_pr(self) -> PullRequest {
        PullRequest {
            id: self.id.parse().unwrap_or_default(),
            number: self.number,
            repo_id: self.repo_id.parse().unwrap_or_default(),
            author_id: self.author_id.parse().unwrap_or_default(),
            title: self.title,
            body: self.body,
            state: match self.state.as_str() {
                "closed" => PrState::Closed,
                "merged" => PrState::Merged,
                _ => PrState::Open,
            },
            head_branch: self.head_branch,
            base_branch: self.base_branch,
            head_sha: self.head_sha,
            is_draft: self.is_draft,
            merged_by: self.merged_by.and_then(|s| s.parse().ok()),
            merge_strategy: self.merge_strategy.as_deref().map(|s| match s {
                "squash" => MergeStrategy::Squash,
                "rebase" => MergeStrategy::Rebase,
                _ => MergeStrategy::Merge,
            }),
            created_at: self.created_at.parse().unwrap_or_default(),
            updated_at: self.updated_at.parse().unwrap_or_default(),
            merged_at: self.merged_at.and_then(|s| s.parse().ok()),
            closed_at: self.closed_at.and_then(|s| s.parse().ok()),
        }
    }
}

#[derive(sqlx::FromRow)]
struct CommentRow {
    id: String,
    pr_id: String,
    author_id: String,
    body: String,
    file_path: Option<String>,
    line: Option<i64>,
    side: Option<String>,
    created_at: String,
    updated_at: String,
}

impl CommentRow {
    fn into_comment(self) -> PrComment {
        PrComment {
            id: self.id.parse().unwrap_or_default(),
            pr_id: self.pr_id.parse().unwrap_or_default(),
            author_id: self.author_id.parse().unwrap_or_default(),
            body: self.body,
            file_path: self.file_path,
            line: self.line,
            side: self.side,
            created_at: self.created_at.parse().unwrap_or_default(),
            updated_at: self.updated_at.parse().unwrap_or_default(),
        }
    }
}

#[derive(sqlx::FromRow)]
struct ReviewRow {
    id: String,
    pr_id: String,
    reviewer_id: String,
    state: String,
    body: Option<String>,
    created_at: String,
}

impl ReviewRow {
    fn into_review(self) -> PrReview {
        PrReview {
            id: self.id.parse().unwrap_or_default(),
            pr_id: self.pr_id.parse().unwrap_or_default(),
            reviewer_id: self.reviewer_id.parse().unwrap_or_default(),
            state: match self.state.as_str() {
                "approved" => ReviewState::Approved,
                "changes_requested" => ReviewState::ChangesRequested,
                _ => ReviewState::Commented,
            },
            body: self.body,
            created_at: self.created_at.parse().unwrap_or_default(),
        }
    }
}
