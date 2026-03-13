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

    #[error("not found: {0}")]
    NotFound(String),

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_display() {
        let err = DeltaError::RepoNotFound("test-repo".into());
        assert_eq!(err.to_string(), "repository not found: test-repo");
    }

    #[test]
    fn test_error_from_io() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file missing");
        let err: DeltaError = io_err.into();
        assert!(err.to_string().contains("file missing"));
    }

    #[test]
    fn test_all_error_variants() {
        assert!(
            !DeltaError::AuthFailed("bad token".into())
                .to_string()
                .is_empty()
        );
        assert!(
            !DeltaError::AuthzDenied("forbidden".into())
                .to_string()
                .is_empty()
        );
        assert!(
            !DeltaError::InvalidRef("bad-ref".into())
                .to_string()
                .is_empty()
        );
        assert!(
            !DeltaError::Conflict("merge conflict".into())
                .to_string()
                .is_empty()
        );
        assert!(
            !DeltaError::Storage("disk full".into())
                .to_string()
                .is_empty()
        );
        assert!(
            !DeltaError::Pipeline("job failed".into())
                .to_string()
                .is_empty()
        );
        assert!(
            !DeltaError::Registry("pkg missing".into())
                .to_string()
                .is_empty()
        );
    }
}
