use delta_ci::PipelineStreams;
use delta_core::DeltaConfig;
use delta_registry::{BlobStore, LfsStore};
use delta_vcs::RepoHost;
use sqlx::SqlitePool;
use std::sync::Arc;

/// Shared application state for the API server.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<DeltaConfig>,
    pub repo_host: Arc<RepoHost>,
    pub blob_store: Arc<BlobStore>,
    pub lfs_store: Arc<LfsStore>,
    pub db: SqlitePool,
    pub pipeline_streams: PipelineStreams,
}

impl AppState {
    pub fn new(config: DeltaConfig, db: SqlitePool) -> Self {
        let repo_host = RepoHost::new(&config.storage.repos_dir);
        let blob_store = BlobStore::new(&config.storage.artifacts_dir);
        let lfs_store = LfsStore::new(config.storage.lfs_dir());
        Self {
            config: Arc::new(config),
            repo_host: Arc::new(repo_host),
            blob_store: Arc::new(blob_store),
            lfs_store: Arc::new(lfs_store),
            db,
            pipeline_streams: delta_ci::new_pipeline_streams(),
        }
    }
}
