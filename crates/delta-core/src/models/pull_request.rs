use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PullRequest {
    pub id: Uuid,
    pub number: i64,
    pub repo_id: Uuid,
    pub author_id: Uuid,
    pub title: String,
    pub body: Option<String>,
    pub state: PrState,
    pub head_branch: String,
    pub base_branch: String,
    pub head_sha: Option<String>,
    pub is_draft: bool,
    pub merged_by: Option<Uuid>,
    pub merge_strategy: Option<MergeStrategy>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub merged_at: Option<DateTime<Utc>>,
    pub closed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrState {
    Open,
    Closed,
    Merged,
}

impl PrState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Closed => "closed",
            Self::Merged => "merged",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MergeStrategy {
    Merge,
    Squash,
    Rebase,
}

impl MergeStrategy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Merge => "merge",
            Self::Squash => "squash",
            Self::Rebase => "rebase",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrComment {
    pub id: Uuid,
    pub pr_id: Uuid,
    pub author_id: Uuid,
    pub body: String,
    /// If set, this is a file-level inline comment.
    pub file_path: Option<String>,
    /// Line number in the diff (for inline comments).
    pub line: Option<i64>,
    /// Side of the diff: "left" (old) or "right" (new).
    pub side: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrReview {
    pub id: Uuid,
    pub pr_id: Uuid,
    pub reviewer_id: Uuid,
    pub state: ReviewState,
    pub body: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewState {
    Approved,
    ChangesRequested,
    Commented,
}

impl ReviewState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Approved => "approved",
            Self::ChangesRequested => "changes_requested",
            Self::Commented => "commented",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusCheck {
    pub id: Uuid,
    pub repo_id: Uuid,
    pub commit_sha: String,
    pub context: String,
    pub state: CheckState,
    pub description: Option<String>,
    pub target_url: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckState {
    Pending,
    Success,
    Failure,
    Error,
}

impl CheckState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Success => "success",
            Self::Failure => "failure",
            Self::Error => "error",
        }
    }
}
