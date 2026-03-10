use delta_core::DeltaConfig;
use delta_vcs::RepoHost;
use std::sync::Arc;

/// Shared application state for the API server.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<DeltaConfig>,
    pub repo_host: Arc<RepoHost>,
}

impl AppState {
    pub fn new(config: DeltaConfig) -> Self {
        let repo_host = RepoHost::new(&config.storage.repos_dir);
        Self {
            config: Arc::new(config),
            repo_host: Arc::new(repo_host),
        }
    }
}
