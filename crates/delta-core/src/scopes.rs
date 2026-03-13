//! Token scope definitions and enforcement for fine-grained API permissions.

use crate::{DeltaError, Result};
use serde::{Deserialize, Serialize};

/// All valid token scopes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    /// Full access to all resources
    All,
    /// Read-only access to repositories
    RepoRead,
    /// Write access to repositories (push, create, delete)
    RepoWrite,
    /// Read pull requests and comments
    PrRead,
    /// Create/update PRs, post comments, submit reviews
    PrWrite,
    /// Read CI pipeline status and logs
    CiRead,
    /// Trigger pipelines, cancel runs
    CiWrite,
    /// Read artifacts and registry packages
    RegistryRead,
    /// Push artifacts and packages
    RegistryWrite,
    /// Manage user profile and SSH keys
    UserProfile,
    /// Manage API tokens
    UserTokens,
    /// Admin operations (collaborators, settings, branch protection)
    Admin,
    /// AI features (code review, PR generation, natural language queries)
    Ai,
}

impl Scope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::All => "*",
            Self::RepoRead => "repo:read",
            Self::RepoWrite => "repo:write",
            Self::PrRead => "pr:read",
            Self::PrWrite => "pr:write",
            Self::CiRead => "ci:read",
            Self::CiWrite => "ci:write",
            Self::RegistryRead => "registry:read",
            Self::RegistryWrite => "registry:write",
            Self::UserProfile => "user:profile",
            Self::UserTokens => "user:tokens",
            Self::Admin => "admin",
            Self::Ai => "ai",
        }
    }

    pub fn parse_str(s: &str) -> Result<Self> {
        match s {
            "*" => Ok(Self::All),
            "repo:read" => Ok(Self::RepoRead),
            "repo:write" => Ok(Self::RepoWrite),
            "pr:read" => Ok(Self::PrRead),
            "pr:write" => Ok(Self::PrWrite),
            "ci:read" => Ok(Self::CiRead),
            "ci:write" => Ok(Self::CiWrite),
            "registry:read" => Ok(Self::RegistryRead),
            "registry:write" => Ok(Self::RegistryWrite),
            "user:profile" => Ok(Self::UserProfile),
            "user:tokens" => Ok(Self::UserTokens),
            "admin" => Ok(Self::Admin),
            "ai" => Ok(Self::Ai),
            // Backward compatibility: map old scopes to new ones
            "read" => Ok(Self::RepoRead),
            "write" => Ok(Self::RepoWrite),
            "repo" => Ok(Self::RepoWrite),
            "user" => Ok(Self::UserProfile),
            _ => Err(DeltaError::AuthFailed(format!("invalid scope: {}", s))),
        }
    }
}

/// A set of scopes parsed from a token's scope string.
#[derive(Debug, Clone)]
pub struct ScopeSet {
    scopes: Vec<Scope>,
    has_all: bool,
}

impl ScopeSet {
    /// Parse a comma-separated scope string into a ScopeSet.
    pub fn parse(scope_string: &str) -> Result<Self> {
        let mut scopes = Vec::new();
        let mut has_all = false;

        for s in scope_string.split(',') {
            let scope = Scope::parse_str(s.trim())?;
            if scope == Scope::All {
                has_all = true;
            }
            scopes.push(scope);
        }

        Ok(Self { scopes, has_all })
    }

    /// Check if this scope set includes the given scope.
    /// The wildcard scope `*` includes all scopes.
    /// Write scopes implicitly include their corresponding read scope.
    pub fn has(&self, required: Scope) -> bool {
        if self.has_all {
            return true;
        }
        self.scopes
            .iter()
            .any(|s| *s == required || Self::implies(*s, required))
    }

    /// Check if scope `a` implies scope `b` (write implies read).
    fn implies(a: Scope, b: Scope) -> bool {
        matches!(
            (a, b),
            (Scope::RepoWrite, Scope::RepoRead)
                | (Scope::PrWrite, Scope::PrRead)
                | (Scope::CiWrite, Scope::CiRead)
                | (Scope::RegistryWrite, Scope::RegistryRead)
                | (Scope::Admin, Scope::RepoRead)
                | (Scope::Admin, Scope::RepoWrite)
                | (Scope::Admin, Scope::PrRead)
                | (Scope::Admin, Scope::PrWrite)
                | (Scope::Admin, Scope::CiRead)
                | (Scope::Admin, Scope::CiWrite)
        )
    }

    /// Return valid scope strings for documentation/validation.
    pub fn valid_scopes() -> &'static [&'static str] {
        &[
            "*",
            "repo:read",
            "repo:write",
            "pr:read",
            "pr:write",
            "ci:read",
            "ci:write",
            "registry:read",
            "registry:write",
            "user:profile",
            "user:tokens",
            "admin",
            "ai",
            // Legacy (still accepted)
            "read",
            "write",
            "repo",
            "user",
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wildcard_has_all() {
        let set = ScopeSet::parse("*").unwrap();
        assert!(set.has(Scope::RepoRead));
        assert!(set.has(Scope::Admin));
        assert!(set.has(Scope::Ai));
    }

    #[test]
    fn test_specific_scopes() {
        let set = ScopeSet::parse("repo:read,pr:write").unwrap();
        assert!(set.has(Scope::RepoRead));
        assert!(set.has(Scope::PrWrite));
        assert!(set.has(Scope::PrRead)); // implied by pr:write
        assert!(!set.has(Scope::Admin));
        assert!(!set.has(Scope::RepoWrite));
    }

    #[test]
    fn test_write_implies_read() {
        let set = ScopeSet::parse("repo:write").unwrap();
        assert!(set.has(Scope::RepoRead));
        assert!(set.has(Scope::RepoWrite));
        assert!(!set.has(Scope::PrRead));
    }

    #[test]
    fn test_admin_implies_repo_and_pr() {
        let set = ScopeSet::parse("admin").unwrap();
        assert!(set.has(Scope::RepoRead));
        assert!(set.has(Scope::RepoWrite));
        assert!(set.has(Scope::PrRead));
        assert!(set.has(Scope::PrWrite));
        assert!(!set.has(Scope::Ai));
    }

    #[test]
    fn test_legacy_scopes() {
        let set = ScopeSet::parse("read,write").unwrap();
        assert!(set.has(Scope::RepoRead));
        assert!(set.has(Scope::RepoWrite));
    }

    #[test]
    fn test_invalid_scope() {
        assert!(ScopeSet::parse("invalid_scope").is_err());
    }
}
