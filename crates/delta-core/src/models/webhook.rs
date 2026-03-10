use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A webhook configuration for a repository.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Webhook {
    pub id: Uuid,
    pub repo_id: Uuid,
    pub url: String,
    pub secret: Option<String>,
    pub events: Vec<WebhookEvent>,
    pub active: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WebhookEvent {
    Push,
    TagCreate,
    TagDelete,
    PullRequest,
    PullRequestReview,
}

/// Payload sent to webhook endpoints on push.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushPayload {
    #[serde(rename = "ref")]
    pub ref_name: String,
    pub before: String,
    pub after: String,
    pub repo_owner: String,
    pub repo_name: String,
    pub pusher: String,
    pub timestamp: DateTime<Utc>,
}
