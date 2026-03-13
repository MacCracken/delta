//! Backup and disaster recovery endpoints.

use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    routing::{get, post},
};
use serde::Serialize;

use crate::extractors::AuthUser;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/status", get(backup_status))
        .route("/snapshot", post(create_snapshot))
}

#[derive(Serialize)]
struct BackupStatus {
    repos_dir: String,
    artifacts_dir: String,
    db_url: String,
    repos_count: i64,
    artifacts_count: i64,
    db_size_bytes: Option<u64>,
    last_backup: Option<String>,
}

async fn backup_status(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
) -> Result<Json<BackupStatus>, (StatusCode, String)> {
    let repos_count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM repositories")
        .fetch_one(&state.db)
        .await
        .unwrap_or(0);

    let artifacts_count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM artifacts")
        .fetch_one(&state.db)
        .await
        .unwrap_or(0);

    // Get database file size
    let db_size = if state.config.storage.db_url.contains("sqlite") {
        let db_path = state
            .config
            .storage
            .db_url
            .strip_prefix("sqlite://")
            .or_else(|| state.config.storage.db_url.strip_prefix("sqlite:"))
            .unwrap_or(&state.config.storage.db_url)
            .split('?')
            .next()
            .unwrap_or("");
        std::fs::metadata(db_path).ok().map(|m| m.len())
    } else {
        None
    };

    Ok(Json(BackupStatus {
        repos_dir: state.config.storage.repos_dir.display().to_string(),
        artifacts_dir: state.config.storage.artifacts_dir.display().to_string(),
        db_url: if state.config.storage.db_url.contains("sqlite") {
            state.config.storage.db_url.clone()
        } else {
            "[redacted]".into()
        },
        repos_count,
        artifacts_count,
        db_size_bytes: db_size,
        last_backup: None,
    }))
}

async fn create_snapshot(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    // For SQLite: use VACUUM INTO to create a consistent snapshot
    if state.config.storage.db_url.contains("sqlite") {
        let backup_dir = state.config.storage.artifacts_dir.join("backups");
        std::fs::create_dir_all(&backup_dir).map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("failed to create backup dir: {}", e),
            )
        })?;

        let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
        let backup_path = backup_dir.join(format!("delta_{}.db", timestamp));

        let sql = format!("VACUUM INTO '{}'", backup_path.display());
        sqlx::query(&sql).execute(&state.db).await.map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("backup failed: {}", e),
            )
        })?;

        let size = std::fs::metadata(&backup_path)
            .map(|m| m.len())
            .unwrap_or(0);

        Ok(Json(serde_json::json!({
            "status": "ok",
            "path": backup_path.display().to_string(),
            "size_bytes": size,
            "created_at": chrono::Utc::now().to_rfc3339(),
        })))
    } else {
        Err((
            StatusCode::BAD_REQUEST,
            "snapshot backup is only available for SQLite databases. For PostgreSQL, use pg_dump."
                .into(),
        ))
    }
}
