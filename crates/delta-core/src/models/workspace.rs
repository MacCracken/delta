//! Workspace model types for remote agent workspaces.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceStatus {
    Active,
    Closed,
    Expired,
}

impl WorkspaceStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Closed => "closed",
            Self::Expired => "expired",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "closed" => Self::Closed,
            "expired" => Self::Expired,
            _ => Self::Active,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workspace {
    pub id: Uuid,
    pub repo_id: String,
    pub creator_id: String,
    pub name: String,
    pub branch: String,
    pub base_branch: String,
    pub base_commit: String,
    pub head_commit: Option<String>,
    pub status: WorkspaceStatus,
    pub ttl_hours: i64,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
