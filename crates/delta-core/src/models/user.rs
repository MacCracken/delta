use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: Uuid,
    pub username: String,
    pub display_name: Option<String>,
    pub email: String,
    pub is_agent: bool,
    pub created_at: DateTime<Utc>,
}

impl User {
    pub fn new(username: impl Into<String>, email: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            username: username.into(),
            display_name: None,
            email: email.into(),
            is_agent: false,
            created_at: Utc::now(),
        }
    }

    pub fn new_agent(username: impl Into<String>, email: impl Into<String>) -> Self {
        let mut user = Self::new(username, email);
        user.is_agent = true;
        user
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_user() {
        let user = User::new("alice", "alice@example.com");
        assert_eq!(user.username, "alice");
        assert_eq!(user.email, "alice@example.com");
        assert!(!user.is_agent);
        assert!(user.display_name.is_none());
    }

    #[test]
    fn test_new_agent() {
        let agent = User::new_agent("bot", "bot@example.com");
        assert!(agent.is_agent);
        assert_eq!(agent.username, "bot");
    }
}
