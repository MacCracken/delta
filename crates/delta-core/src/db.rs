pub mod artifact;
pub mod audit;
pub mod branch_protection;
pub mod pipeline;
pub mod pull_request;
pub mod release;
pub mod repo;
pub mod secret;
pub mod status_check;
pub mod user;
pub mod webhook;

use crate::Result;
use sqlx::SqlitePool;

/// Initialize the database connection pool and run migrations.
pub async fn init_pool(db_url: &str) -> Result<SqlitePool> {
    let url = db_url.strip_prefix("sqlite://").unwrap_or(db_url);

    // Ensure parent directory exists
    if let Some(parent) = std::path::Path::new(url).parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }

    let pool = SqlitePool::connect(db_url)
        .await
        .map_err(|e| crate::DeltaError::Storage(e.to_string()))?;

    // Run migrations
    for migration in [
        include_str!("../migrations/001_initial.sql"),
        include_str!("../migrations/002_git_protocol.sql"),
        include_str!("../migrations/003_pull_requests.sql"),
        include_str!("../migrations/004_cicd.sql"),
    ] {
        sqlx::query(migration)
            .execute(&pool)
            .await
            .map_err(|e| crate::DeltaError::Storage(e.to_string()))?;
    }

    tracing::info!("database initialized");
    Ok(pool)
}
