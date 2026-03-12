use crate::{DeltaError, Result};
use sqlx::SqlitePool;
use uuid::Uuid;

/// Get all active webhooks for a repo that listen for a given event.
pub async fn get_for_event(
    pool: &SqlitePool,
    repo_id: &str,
    event: &str,
) -> Result<Vec<WebhookRow>> {
    let rows = sqlx::query_as::<_, WebhookRow>(
        "SELECT * FROM webhooks WHERE repo_id = ? AND active = TRUE",
    )
    .bind(repo_id)
    .fetch_all(pool)
    .await
    .map_err(|e| DeltaError::Storage(e.to_string()))?;

    // Filter by event (events stored as JSON array — parse for exact match)
    Ok(rows
        .into_iter()
        .filter(|w| {
            serde_json::from_str::<Vec<String>>(&w.events)
                .map(|events| events.iter().any(|e| e == event))
                .unwrap_or(false)
        })
        .collect())
}

/// Create a webhook.
pub async fn create(
    pool: &SqlitePool,
    repo_id: &str,
    url: &str,
    secret: Option<&str>,
    events: &str,
) -> Result<String> {
    let id = Uuid::new_v4().to_string();

    sqlx::query("INSERT INTO webhooks (id, repo_id, url, secret, events) VALUES (?, ?, ?, ?, ?)")
        .bind(&id)
        .bind(repo_id)
        .bind(url)
        .bind(secret)
        .bind(events)
        .execute(pool)
        .await
        .map_err(|e| DeltaError::Storage(e.to_string()))?;

    Ok(id)
}

/// List webhooks for a repo.
pub async fn list_for_repo(pool: &SqlitePool, repo_id: &str) -> Result<Vec<WebhookRow>> {
    let rows = sqlx::query_as::<_, WebhookRow>(
        "SELECT * FROM webhooks WHERE repo_id = ? ORDER BY created_at DESC",
    )
    .bind(repo_id)
    .fetch_all(pool)
    .await
    .map_err(|e| DeltaError::Storage(e.to_string()))?;

    Ok(rows)
}

/// Delete a webhook.
pub async fn delete(pool: &SqlitePool, webhook_id: &str, repo_id: &str) -> Result<()> {
    let result = sqlx::query("DELETE FROM webhooks WHERE id = ? AND repo_id = ?")
        .bind(webhook_id)
        .bind(repo_id)
        .execute(pool)
        .await
        .map_err(|e| DeltaError::Storage(e.to_string()))?;

    if result.rows_affected() == 0 {
        return Err(DeltaError::RepoNotFound("webhook not found".into()));
    }
    Ok(())
}

/// Record a webhook delivery.
pub async fn record_delivery(
    pool: &SqlitePool,
    webhook_id: &str,
    event: &str,
    payload: &str,
    response_status: Option<i32>,
    response_body: Option<&str>,
) -> Result<()> {
    let id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO webhook_deliveries (id, webhook_id, event, payload, response_status, response_body)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(webhook_id)
    .bind(event)
    .bind(payload)
    .bind(response_status)
    .bind(response_body)
    .execute(pool)
    .await
    .map_err(|e| DeltaError::Storage(e.to_string()))?;

    Ok(())
}

#[derive(Debug, sqlx::FromRow)]
pub struct WebhookRow {
    pub id: String,
    pub repo_id: String,
    pub url: String,
    pub secret: Option<String>,
    pub events: String,
    pub active: bool,
    pub created_at: String,
}
