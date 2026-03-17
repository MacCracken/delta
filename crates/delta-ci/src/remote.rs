//! Remote job dispatch — types and logic for self-hosted runner execution.
//!
//! When a workflow job has `runs_on` starting with "self-hosted", the job is
//! not executed locally. Instead, a serializable payload is enqueued for a
//! remote runner to pick up, execute, and report results back.

use crate::workflow::Job;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Serializable job payload sent to self-hosted runners.
/// Contains everything the runner needs to execute the job.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobPayload {
    /// The queue entry ID (used to report completion).
    pub queue_id: String,
    /// The job_run ID in the Delta database.
    pub job_run_id: String,
    /// Pipeline ID this job belongs to.
    pub pipeline_id: String,
    /// Repository ID.
    pub repo_id: String,
    /// Human-readable job name.
    pub job_name: String,
    /// The steps to execute in order.
    pub steps: Vec<StepPayload>,
    /// Environment variables to inject (DELTA_* vars, MATRIX_* vars only — secrets excluded).
    pub env: HashMap<String, String>,
    /// Git clone URL for the repository.
    pub clone_url: Option<String>,
    /// Commit SHA to check out.
    pub commit_sha: String,
}

/// A single step within a job payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepPayload {
    pub name: String,
    pub run: String,
}

/// Result reported back by a runner after executing a job.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobReport {
    /// The queue entry ID.
    pub queue_id: String,
    /// Whether all steps passed.
    pub success: bool,
    /// Per-step results.
    pub steps: Vec<StepReport>,
}

/// Result of a single step execution on the runner.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepReport {
    pub name: String,
    pub exit_code: i32,
    pub output: String,
}

/// Parse the `runs_on` field and determine if a job targets a self-hosted runner.
/// Returns `Some(labels)` if the job should be dispatched remotely, `None` if local.
///
/// Format: `"self-hosted"` or `"self-hosted, label1, label2"`
pub fn parse_self_hosted(runs_on: Option<&str>) -> Option<Vec<String>> {
    let runs_on = runs_on?;
    let parts: Vec<&str> = runs_on.split(',').map(|s| s.trim()).collect();

    if parts.first().map(|s| s.to_lowercase()) != Some("self-hosted".to_string()) {
        return None;
    }

    // Labels are everything after "self-hosted"
    let labels: Vec<String> = parts
        .into_iter()
        .skip(1)
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
        .collect();

    Some(labels)
}

/// Build a `JobPayload` from a workflow job definition and execution context.
#[allow(clippy::too_many_arguments)]
pub fn build_payload(
    queue_id: &str,
    job_run_id: &str,
    pipeline_id: &str,
    repo_id: &str,
    job_name: &str,
    job: &Job,
    env: &HashMap<String, String>,
    commit_sha: &str,
) -> JobPayload {
    let steps = job
        .steps
        .iter()
        .enumerate()
        .filter_map(|(i, step)| {
            step.run.as_ref().map(|cmd| StepPayload {
                name: step.name.clone().unwrap_or_else(|| format!("step-{}", i)),
                run: cmd.clone(),
            })
        })
        .collect();

    JobPayload {
        queue_id: queue_id.to_string(),
        job_run_id: job_run_id.to_string(),
        pipeline_id: pipeline_id.to_string(),
        repo_id: repo_id.to_string(),
        job_name: job_name.to_string(),
        steps,
        env: env.clone(),
        clone_url: None,
        commit_sha: commit_sha.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::Step;

    #[test]
    fn test_parse_self_hosted_none() {
        assert!(parse_self_hosted(None).is_none());
        assert!(parse_self_hosted(Some("local")).is_none());
        assert!(parse_self_hosted(Some("docker://alpine")).is_none());
        assert!(parse_self_hosted(Some("ubuntu-latest")).is_none());
    }

    #[test]
    fn test_parse_self_hosted_no_labels() {
        let labels = parse_self_hosted(Some("self-hosted")).unwrap();
        assert!(labels.is_empty());
    }

    #[test]
    fn test_parse_self_hosted_with_labels() {
        let labels = parse_self_hosted(Some("self-hosted, linux, gpu")).unwrap();
        assert_eq!(labels, vec!["linux", "gpu"]);
    }

    #[test]
    fn test_parse_self_hosted_case_insensitive() {
        let labels = parse_self_hosted(Some("Self-Hosted, arm64")).unwrap();
        assert_eq!(labels, vec!["arm64"]);
    }

    #[test]
    fn test_build_payload() {
        let job = Job {
            name: Some("Test".into()),
            runs_on: Some("self-hosted".into()),
            needs: vec![],
            steps: vec![
                Step {
                    name: Some("Build".into()),
                    run: Some("cargo build".into()),
                    uses: None,
                    with: HashMap::new(),
                },
                Step {
                    name: None,
                    run: Some("cargo test".into()),
                    uses: None,
                    with: HashMap::new(),
                },
            ],
            uses: None,
            with: HashMap::new(),
            strategy: None,
        };

        let env = HashMap::from([("DELTA_REPO_ID".into(), "repo-1".into())]);
        let payload = build_payload("q1", "j1", "p1", "r1", "test", &job, &env, "abc123");

        assert_eq!(payload.steps.len(), 2);
        assert_eq!(payload.steps[0].name, "Build");
        assert_eq!(payload.steps[1].name, "step-1");
        assert_eq!(payload.commit_sha, "abc123");
        assert_eq!(payload.env.get("DELTA_REPO_ID").unwrap(), "repo-1");
    }

    #[test]
    fn test_job_payload_serde_roundtrip() {
        let payload = JobPayload {
            queue_id: "q1".into(),
            job_run_id: "j1".into(),
            pipeline_id: "p1".into(),
            repo_id: "r1".into(),
            job_name: "test".into(),
            steps: vec![StepPayload {
                name: "echo".into(),
                run: "echo hello".into(),
            }],
            env: HashMap::new(),
            clone_url: None,
            commit_sha: "abc".into(),
        };

        let json = serde_json::to_string(&payload).unwrap();
        let deserialized: JobPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.queue_id, "q1");
        assert_eq!(deserialized.steps.len(), 1);
    }
}
