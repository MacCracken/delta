use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Status of a pipeline run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PipelineStatus {
    Queued,
    Running,
    Passed,
    Failed,
    Cancelled,
}

/// A single pipeline run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineRun {
    pub id: Uuid,
    pub workflow_name: String,
    pub repo_owner: String,
    pub repo_name: String,
    pub commit_sha: String,
    pub status: PipelineStatus,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pipeline_status_serde() {
        let json = serde_json::to_string(&PipelineStatus::Running).unwrap();
        assert_eq!(json, "\"running\"");
        let parsed: PipelineStatus = serde_json::from_str("\"passed\"").unwrap();
        assert_eq!(parsed, PipelineStatus::Passed);
    }

    #[test]
    fn test_pipeline_run_new() {
        let run = PipelineRun {
            id: Uuid::new_v4(),
            workflow_name: "CI".into(),
            repo_owner: "alice".into(),
            repo_name: "delta".into(),
            commit_sha: "abc123".into(),
            status: PipelineStatus::Queued,
            started_at: None,
            finished_at: None,
        };
        assert_eq!(run.status, PipelineStatus::Queued);
        assert!(run.started_at.is_none());
    }
}
