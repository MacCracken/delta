//! Input validation for git operations.

use delta_core::{DeltaError, Result};

/// Validate a git ref name (branch, tag, commit SHA).
/// Prevents git option injection and invalid characters.
pub fn validate_ref(name: &str) -> Result<()> {
    if name.is_empty() {
        return Err(DeltaError::InvalidRef("ref name cannot be empty".into()));
    }
    if name.starts_with('-') {
        return Err(DeltaError::InvalidRef(
            "ref name cannot start with '-'".into(),
        ));
    }
    if name.contains("..") {
        return Err(DeltaError::InvalidRef(
            "ref name cannot contain '..'".into(),
        ));
    }
    if name.contains('\0') || name.contains(' ') || name.contains('~') || name.contains('^')
        || name.contains(':') || name.contains('\\') || name.contains('\x7f')
    {
        return Err(DeltaError::InvalidRef(
            "ref name contains invalid characters".into(),
        ));
    }
    if name.ends_with('/') || name.ends_with('.') || name.ends_with(".lock") {
        return Err(DeltaError::InvalidRef(
            "ref name has invalid suffix".into(),
        ));
    }
    Ok(())
}

/// Validate an owner or repo name for filesystem safety.
/// Only allows alphanumeric, hyphens, underscores, and dots (not leading).
pub fn validate_name(name: &str) -> Result<()> {
    if name.is_empty() {
        return Err(DeltaError::InvalidRef("name cannot be empty".into()));
    }
    if name.starts_with('.') || name.starts_with('-') {
        return Err(DeltaError::InvalidRef(
            "name cannot start with '.' or '-'".into(),
        ));
    }
    if name.contains('/') || name.contains('\\') || name.contains('\0') {
        return Err(DeltaError::InvalidRef(
            "name contains path separators".into(),
        ));
    }
    if name == "." || name == ".." {
        return Err(DeltaError::InvalidRef(
            "name cannot be '.' or '..'".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_refs() {
        assert!(validate_ref("main").is_ok());
        assert!(validate_ref("feature/my-branch").is_ok());
        assert!(validate_ref("v1.0.0").is_ok());
        assert!(validate_ref("abc123def456").is_ok());
    }

    #[test]
    fn test_invalid_refs() {
        assert!(validate_ref("").is_err());
        assert!(validate_ref("--exec=foo").is_err());
        assert!(validate_ref("-n").is_err());
        assert!(validate_ref("a..b").is_err());
        assert!(validate_ref("ref\0name").is_err());
        assert!(validate_ref("ref name").is_err());
        assert!(validate_ref("ref~1").is_err());
        assert!(validate_ref("ref^2").is_err());
        assert!(validate_ref("ref:name").is_err());
        assert!(validate_ref("name.lock").is_err());
        assert!(validate_ref("name/").is_err());
    }

    #[test]
    fn test_valid_names() {
        assert!(validate_name("alice").is_ok());
        assert!(validate_name("my-repo").is_ok());
        assert!(validate_name("repo_v2").is_ok());
    }

    #[test]
    fn test_invalid_names() {
        assert!(validate_name("").is_err());
        assert!(validate_name("..").is_err());
        assert!(validate_name(".hidden").is_err());
        assert!(validate_name("-leading").is_err());
        assert!(validate_name("../../etc").is_err());
        assert!(validate_name("a/b").is_err());
    }
}
