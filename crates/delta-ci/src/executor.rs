//! Job executor — runs workflow steps as shell commands.
//!
//! Each step's `run` field is executed as a shell command.
//! Steps run sequentially within a job. Jobs respect `needs` ordering.

use crate::workflow::{Job, Workflow};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;
use std::process::Stdio;
use tokio::process::Command;

/// Result of executing a single step.
#[derive(Debug, Clone)]
pub struct StepResult {
    pub name: String,
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// Result of executing a job (all its steps).
#[derive(Debug, Clone)]
pub struct JobResult {
    pub job_name: String,
    pub success: bool,
    pub steps: Vec<StepResult>,
}

/// Execute a single job's steps in order.
pub async fn execute_job(
    job_name: &str,
    job: &Job,
    work_dir: &Path,
    env_vars: &HashMap<String, String>,
) -> JobResult {
    let mut steps = Vec::new();
    let mut all_passed = true;

    for step in &job.steps {
        let Some(cmd) = &step.run else {
            continue;
        };

        let step_name = step
            .name
            .clone()
            .unwrap_or_else(|| format!("step-{}", steps.len()));

        let result = run_step(&step_name, cmd, work_dir, env_vars).await;

        if result.exit_code != 0 {
            all_passed = false;
            steps.push(result);
            break; // Stop on first failure
        }

        steps.push(result);
    }

    JobResult {
        job_name: job_name.to_string(),
        success: all_passed,
        steps,
    }
}

/// Resolve job execution order based on `needs` dependencies.
/// Returns jobs in topologically sorted order.
pub fn resolve_job_order(workflow: &Workflow) -> Result<Vec<String>, String> {
    let mut in_degree: HashMap<&str, usize> = HashMap::new();
    let mut dependents: HashMap<&str, Vec<&str>> = HashMap::new();

    for (name, job) in &workflow.jobs {
        in_degree.entry(name.as_str()).or_insert(0);
        for dep in &job.needs {
            if !workflow.jobs.contains_key(dep.as_str()) {
                return Err(format!("job '{}' depends on unknown job '{}'", name, dep));
            }
            *in_degree.entry(name.as_str()).or_insert(0) += 1;
            dependents
                .entry(dep.as_str())
                .or_default()
                .push(name.as_str());
        }
    }

    let mut queue: VecDeque<&str> = in_degree
        .iter()
        .filter(|(_, deg)| **deg == 0)
        .map(|(name, _)| *name)
        .collect();

    let mut order = Vec::new();
    let mut visited = HashSet::new();

    while let Some(name) = queue.pop_front() {
        if !visited.insert(name) {
            continue;
        }
        order.push(name.to_string());

        if let Some(deps) = dependents.get(name) {
            for &dep in deps {
                if let Some(deg) = in_degree.get_mut(dep) {
                    *deg = deg.saturating_sub(1);
                    if *deg == 0 {
                        queue.push_back(dep);
                    }
                }
            }
        }
    }

    if order.len() != workflow.jobs.len() {
        return Err("circular dependency detected in job graph".into());
    }

    Ok(order)
}

async fn run_step(
    name: &str,
    cmd: &str,
    work_dir: &Path,
    env_vars: &HashMap<String, String>,
) -> StepResult {
    tracing::info!(step = name, "executing step");

    let output = Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .current_dir(work_dir)
        .envs(env_vars)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await;

    match output {
        Ok(out) => StepResult {
            name: name.to_string(),
            exit_code: out.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&out.stdout).to_string(),
            stderr: String::from_utf8_lossy(&out.stderr).to_string(),
        },
        Err(e) => StepResult {
            name: name.to_string(),
            exit_code: -1,
            stdout: String::new(),
            stderr: format!("failed to execute: {}", e),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::{Job, Step, Workflow};

    fn make_workflow() -> Workflow {
        let mut jobs = HashMap::new();
        jobs.insert(
            "build".to_string(),
            Job {
                name: Some("Build".into()),
                runs_on: None,
                needs: vec![],
                steps: vec![Step {
                    name: Some("Compile".into()),
                    run: Some("echo building".into()),
                    uses: None,
                    with: HashMap::new(),
                }],
            },
        );
        jobs.insert(
            "test".to_string(),
            Job {
                name: Some("Test".into()),
                runs_on: None,
                needs: vec!["build".into()],
                steps: vec![Step {
                    name: Some("Run tests".into()),
                    run: Some("echo testing".into()),
                    uses: None,
                    with: HashMap::new(),
                }],
            },
        );
        jobs.insert(
            "deploy".to_string(),
            Job {
                name: Some("Deploy".into()),
                runs_on: None,
                needs: vec!["test".into()],
                steps: vec![],
            },
        );
        Workflow {
            name: "CI".into(),
            on: vec![],
            jobs,
        }
    }

    #[test]
    fn test_resolve_job_order() {
        let wf = make_workflow();
        let order = resolve_job_order(&wf).unwrap();
        assert_eq!(order.len(), 3);

        let build_pos = order.iter().position(|s| s == "build").unwrap();
        let test_pos = order.iter().position(|s| s == "test").unwrap();
        let deploy_pos = order.iter().position(|s| s == "deploy").unwrap();
        assert!(build_pos < test_pos);
        assert!(test_pos < deploy_pos);
    }

    #[test]
    fn test_circular_dependency_detected() {
        let mut jobs = HashMap::new();
        jobs.insert(
            "a".to_string(),
            Job {
                name: None,
                runs_on: None,
                needs: vec!["b".into()],
                steps: vec![],
            },
        );
        jobs.insert(
            "b".to_string(),
            Job {
                name: None,
                runs_on: None,
                needs: vec!["a".into()],
                steps: vec![],
            },
        );
        let wf = Workflow {
            name: "bad".into(),
            on: vec![],
            jobs,
        };
        assert!(resolve_job_order(&wf).is_err());
    }

    #[tokio::test]
    async fn test_execute_job() {
        let job = Job {
            name: Some("Test".into()),
            runs_on: None,
            needs: vec![],
            steps: vec![
                Step {
                    name: Some("Echo".into()),
                    run: Some("echo hello".into()),
                    uses: None,
                    with: HashMap::new(),
                },
                Step {
                    name: Some("Check".into()),
                    run: Some("true".into()),
                    uses: None,
                    with: HashMap::new(),
                },
            ],
        };

        let result = execute_job("test", &job, std::path::Path::new("/tmp"), &HashMap::new()).await;
        assert!(result.success);
        assert_eq!(result.steps.len(), 2);
        assert!(result.steps[0].stdout.contains("hello"));
    }

    #[tokio::test]
    async fn test_execute_job_fails_on_error() {
        let job = Job {
            name: None,
            runs_on: None,
            needs: vec![],
            steps: vec![
                Step {
                    name: Some("Fail".into()),
                    run: Some("false".into()),
                    uses: None,
                    with: HashMap::new(),
                },
                Step {
                    name: Some("Never runs".into()),
                    run: Some("echo nope".into()),
                    uses: None,
                    with: HashMap::new(),
                },
            ],
        };

        let result = execute_job("test", &job, std::path::Path::new("/tmp"), &HashMap::new()).await;
        assert!(!result.success);
        assert_eq!(result.steps.len(), 1); // Stopped after failure
    }
}
