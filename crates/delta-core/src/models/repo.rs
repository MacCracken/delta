use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Repository {
    pub id: Uuid,
    pub owner: String,
    pub name: String,
    pub description: Option<String>,
    pub visibility: Visibility,
    pub default_branch: String,
    pub forked_from: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Visibility {
    Public,
    Private,
    Internal,
}

impl Visibility {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Private => "private",
            Self::Internal => "internal",
        }
    }
}

impl Repository {
    pub fn new(owner: impl Into<String>, name: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            owner: owner.into(),
            name: name.into(),
            description: None,
            visibility: Visibility::Private,
            default_branch: "main".into(),
            forked_from: None,
            created_at: now,
            updated_at: now,
        }
    }

    pub fn full_name(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_repo_defaults() {
        let repo = Repository::new("alice", "delta");
        assert_eq!(repo.owner, "alice");
        assert_eq!(repo.name, "delta");
        assert_eq!(repo.default_branch, "main");
        assert_eq!(repo.visibility, Visibility::Private);
        assert!(repo.description.is_none());
    }

    #[test]
    fn test_full_name() {
        let repo = Repository::new("alice", "delta");
        assert_eq!(repo.full_name(), "alice/delta");
    }

    #[test]
    fn test_visibility_as_str() {
        assert_eq!(Visibility::Public.as_str(), "public");
        assert_eq!(Visibility::Private.as_str(), "private");
        assert_eq!(Visibility::Internal.as_str(), "internal");
    }
}
