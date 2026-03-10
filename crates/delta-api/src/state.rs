use delta_core::DeltaConfig;
use delta_registry::BlobStore;
use delta_vcs::RepoHost;
use sqlx::SqlitePool;
use std::sync::Arc;

/// Shared application state for the API server.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<DeltaConfig>,
    pub repo_host: Arc<RepoHost>,
    pub blob_store: Arc<BlobStore>,
    pub db: SqlitePool,
}

impl AppState {
    pub fn new(config: DeltaConfig, db: SqlitePool) -> Self {
        let repo_host = RepoHost::new(&config.storage.repos_dir);
        let blob_store = BlobStore::new(&config.storage.artifacts_dir);
        Self {
            config: Arc::new(config),
            repo_host: Arc::new(repo_host),
            blob_store: Arc::new(blob_store),
            db,
        }
    }
}
