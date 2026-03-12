use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Role a collaborator holds on a repository.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CollaboratorRole {
    Read,
    Write,
    Admin,
}

impl CollaboratorRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::Admin => "admin",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "read" => Some(Self::Read),
            "write" => Some(Self::Write),
            "admin" => Some(Self::Admin),
            _ => None,
        }
    }

    /// Whether this role grants at least the given permission level.
    pub fn has(self, required: Self) -> bool {
        self >= required
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Collaborator {
    pub id: Uuid,
    pub repo_id: Uuid,
    pub user_id: Uuid,
    pub role: CollaboratorRole,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_role_ordering() {
        assert!(CollaboratorRole::Admin > CollaboratorRole::Write);
        assert!(CollaboratorRole::Write > CollaboratorRole::Read);
    }

    #[test]
    fn test_role_has() {
        assert!(CollaboratorRole::Admin.has(CollaboratorRole::Read));
        assert!(CollaboratorRole::Admin.has(CollaboratorRole::Write));
        assert!(CollaboratorRole::Admin.has(CollaboratorRole::Admin));
        assert!(CollaboratorRole::Write.has(CollaboratorRole::Read));
        assert!(CollaboratorRole::Write.has(CollaboratorRole::Write));
        assert!(!CollaboratorRole::Write.has(CollaboratorRole::Admin));
        assert!(!CollaboratorRole::Read.has(CollaboratorRole::Write));
    }

    #[test]
    fn test_role_roundtrip() {
        for role in [
            CollaboratorRole::Read,
            CollaboratorRole::Write,
            CollaboratorRole::Admin,
        ] {
            assert_eq!(CollaboratorRole::parse(role.as_str()), Some(role));
        }
        assert_eq!(CollaboratorRole::parse("invalid"), None);
    }
}
