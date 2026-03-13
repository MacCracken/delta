pub mod ark_package;
pub mod artifact;
pub mod audit;
pub mod branch_protection;
pub mod collaborator;
pub mod download_stats;
pub mod encryption;
pub mod federation;
pub mod lfs;
pub mod oci;
pub mod pipeline;
pub mod pull_request;
pub mod release;
pub mod repo;
pub mod retention;
pub mod search;
pub mod secret;
pub mod signing;
pub mod ssh_key;
pub mod status_check;
pub mod user;
pub mod webhook;

use crate::Result;
use sqlx::SqlitePool;
use sqlx::sqlite::SqliteConnectOptions;
use std::str::FromStr;

/// Initialize the database connection pool and run migrations.
pub async fn init_pool(db_url: &str) -> Result<SqlitePool> {
    let url = db_url.strip_prefix("sqlite://").unwrap_or(db_url);

    // Ensure parent directory exists
    if let Some(parent) = std::path::Path::new(url).parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }

    let options = SqliteConnectOptions::from_str(db_url)
        .map_err(|e| crate::DeltaError::Storage(e.to_string()))?
        .create_if_missing(true);

    let pool = SqlitePool::connect_with(options)
        .await
        .map_err(|e| crate::DeltaError::Storage(e.to_string()))?;

    // Run migrations
    for migration in [
        include_str!("../migrations/001_initial.sql"),
        include_str!("../migrations/002_git_protocol.sql"),
        include_str!("../migrations/003_pull_requests.sql"),
        include_str!("../migrations/004_cicd.sql"),
        include_str!("../migrations/005_registry.sql"),
        include_str!("../migrations/006_collaborators.sql"),
        include_str!("../migrations/007_forks_and_templates.sql"),
        include_str!("../migrations/008_lfs.sql"),
        include_str!("../migrations/009_cascade_fixes.sql"),
        include_str!("../migrations/010_search.sql"),
        include_str!("../migrations/011_federation.sql"),
        include_str!("../migrations/012_encryption.sql"),
    ] {
        sqlx::query(migration)
            .execute(&pool)
            .await
            .map_err(|e| crate::DeltaError::Storage(e.to_string()))?;
    }

    tracing::info!("database initialized");
    Ok(pool)
}
