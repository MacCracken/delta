use thiserror::Error;

#[derive(Debug, Error)]
pub enum DeltaError {
    #[error("repository not found: {0}")]
    RepoNotFound(String),

    #[error("authentication failed: {0}")]
    AuthFailed(String),

    #[error("authorization denied: {0}")]
    AuthzDenied(String),

    #[error("invalid reference: {0}")]
    InvalidRef(String),

    #[error("conflict: {0}")]
    Conflict(String),

    #[error("storage error: {0}")]
    Storage(String),

    #[error("pipeline error: {0}")]
    Pipeline(String),

    #[error("registry error: {0}")]
    Registry(String),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Serde(#[from] serde_json::Error),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

pub type Result<T> = std::result::Result<T, DeltaError>;
