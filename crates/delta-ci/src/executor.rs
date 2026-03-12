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

/// A concrete job instance produced by matrix expansion.
/// For non-matrix jobs, there is exactly one instance with an empty `matrix_values`.
#[derive(Debug, Clone)]
pub struct ExpandedJob {
    /// Original job key, e.g. "test".
    pub key: String,
    /// Display name including matrix values, e.g. "test (linux, stable)".
    pub display_name: String,
    /// The job definition to execute.
    pub job: Job,
    /// Matrix dimension values for this instance (injected as MATRIX_* env vars).
    pub matrix_values: HashMap<String, String>,
    /// Whether fail-fast is enabled for this matrix group.
    pub fail_fast: bool,
}

/// Maximum number of matrix combinations allowed to prevent resource exhaustion.
const MAX_MATRIX_COMBINATIONS: usize = 256;

/// Compute the Cartesian product of matrix dimensions.
/// Returns an error if the total number of combinations exceeds MAX_MATRIX_COMBINATIONS.
fn cartesian_product(
    matrix: &HashMap<String, Vec<String>>,
) -> Result<Vec<HashMap<String, String>>, String> {
    let keys: Vec<&String> = matrix.keys().collect();
    if keys.is_empty() {
        return Ok(vec![HashMap::new()]);
    }

    // Pre-check total combination count to avoid unbounded allocation
    let total: usize = keys
        .iter()
        .map(|k| matrix[*k].len().max(1))
        .try_fold(1usize, |acc, n| acc.checked_mul(n))
        .unwrap_or(usize::MAX);
    if total > MAX_MATRIX_COMBINATIONS {
        return Err(format!(
            "matrix produces {} combinations, exceeding limit of {}",
            total, MAX_MATRIX_COMBINATIONS
        ));
    }

    let mut combos: Vec<HashMap<String, String>> = vec![HashMap::new()];
    for key in &keys {
        let values = &matrix[*key];
        let mut new_combos = Vec::new();
        for combo in &combos {
            for val in values {
                let mut c = combo.clone();
                c.insert((*key).clone(), val.clone());
                new_combos.push(c);
            }
        }
        combos = new_combos;
    }
    Ok(combos)
}

/// Expand a single job into one or more concrete instances based on its
/// matrix strategy. Non-matrix jobs produce a single instance.
pub fn expand_matrix_job(key: &str, job: &Job) -> Result<Vec<ExpandedJob>, String> {
    let Some(strategy) = &job.strategy else {
        return Ok(vec![ExpandedJob {
            key: key.to_string(),
            display_name: key.to_string(),
            job: job.clone(),
            matrix_values: HashMap::new(),
            fail_fast: false,
        }]);
    };

    let combos = cartesian_product(&strategy.matrix)?;
    if combos.is_empty() || (combos.len() == 1 && combos[0].is_empty()) {
        return Ok(vec![ExpandedJob {
            key: key.to_string(),
            display_name: key.to_string(),
            job: job.clone(),
            matrix_values: HashMap::new(),
            fail_fast: strategy.fail_fast,
        }]);
    }

    // Sort keys for deterministic display names
    let mut sorted_keys: Vec<&String> = strategy.matrix.keys().collect();
    sorted_keys.sort();

    Ok(combos
        .into_iter()
        .map(|values| {
            let label: Vec<&str> = sorted_keys
                .iter()
                .filter_map(|k| values.get(*k).map(|v| v.as_str()))
                .collect();
            let display_name = format!("{} ({})", key, label.join(", "));

            ExpandedJob {
                key: key.to_string(),
                display_name,
                job: job.clone(),
                matrix_values: values,
                fail_fast: strategy.fail_fast,
            }
        })
        .collect())
}

/// Expand all jobs in a workflow, replacing matrix jobs with their
/// concrete instances. Returns (expanded jobs map, execution order).
pub fn expand_workflow_matrices(
    workflow: &Workflow,
) -> Result<(HashMap<String, ExpandedJob>, Vec<String>), String> {
    // First expand all jobs
    let mut expanded: HashMap<String, ExpandedJob> = HashMap::new();
    // Track which expanded keys come from which original key
    let mut original_to_expanded: HashMap<String, Vec<String>> = HashMap::new();

    for (key, job) in &workflow.jobs {
        let instances = expand_matrix_job(key, job)?;
        let mut instance_keys = Vec::new();
        for inst in instances {
            let inst_key = inst.display_name.clone();
            instance_keys.push(inst_key.clone());
            expanded.insert(inst_key, inst);
        }
        original_to_expanded.insert(key.clone(), instance_keys);
    }

    // Rebuild needs: if job A needs job B, and B expanded into B(linux) + B(macos),
    // then each instance of A needs ALL instances of B.
    for inst in expanded.values_mut() {
        let original_needs = inst.job.needs.clone();
        let mut new_needs = Vec::new();
        for dep in &original_needs {
            if let Some(dep_instances) = original_to_expanded.get(dep) {
                new_needs.extend(dep_instances.clone());
            } else {
                return Err(format!(
                    "job '{}' depends on unknown job '{}'",
                    inst.key, dep
                ));
            }
        }
        inst.job.needs = new_needs;
    }

    // Build a temporary Workflow with expanded jobs for topological sort
    let expanded_workflow = Workflow {
        name: workflow.name.clone(),
        on: workflow.on.clone(),
        jobs: expanded
            .iter()
            .map(|(k, v)| (k.clone(), v.job.clone()))
            .collect(),
    };

    let order = resolve_job_order(&expanded_workflow)?;
    Ok((expanded, order))
}

/// Maximum time a single step can run before being killed.
const STEP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30 * 60); // 30 minutes

async fn run_step(
    name: &str,
    cmd: &str,
    work_dir: &Path,
    env_vars: &HashMap<String, String>,
) -> StepResult {
    tracing::info!(step = name, "executing step");

    let output = tokio::time::timeout(
        STEP_TIMEOUT,
        Command::new("sh")
            .arg("-c")
            .arg(cmd)
            .current_dir(work_dir)
            .envs(env_vars)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output(),
    )
    .await;

    match output {
        Ok(Ok(out)) => StepResult {
            name: name.to_string(),
            exit_code: out.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&out.stdout).to_string(),
            stderr: String::from_utf8_lossy(&out.stderr).to_string(),
        },
        Ok(Err(e)) => StepResult {
            name: name.to_string(),
            exit_code: -1,
            stdout: String::new(),
            stderr: format!("failed to execute: {}", e),
        },
        Err(_) => StepResult {
            name: name.to_string(),
            exit_code: -1,
            stdout: String::new(),
            stderr: format!("step timed out after {} seconds", STEP_TIMEOUT.as_secs()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::{Job, MatrixStrategy, Step, Workflow};

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
                uses: None,
                with: HashMap::new(),
                strategy: None,
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
                uses: None,
                with: HashMap::new(),
                strategy: None,
            },
        );
        jobs.insert(
            "deploy".to_string(),
            Job {
                name: Some("Deploy".into()),
                runs_on: None,
                needs: vec!["test".into()],
                steps: vec![],
                uses: None,
                with: HashMap::new(),
                strategy: None,
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
                uses: None,
                with: HashMap::new(),
                strategy: None,
            },
        );
        jobs.insert(
            "b".to_string(),
            Job {
                name: None,
                runs_on: None,
                needs: vec!["a".into()],
                steps: vec![],
                uses: None,
                with: HashMap::new(),
                strategy: None,
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
            uses: None,
            with: HashMap::new(),
            strategy: None,
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
            uses: None,
            with: HashMap::new(),
            strategy: None,
        };

        let result = execute_job("test", &job, std::path::Path::new("/tmp"), &HashMap::new()).await;
        assert!(!result.success);
        assert_eq!(result.steps.len(), 1); // Stopped after failure
    }

    #[test]
    fn test_cartesian_product_empty() {
        let m = HashMap::new();
        let combos = cartesian_product(&m).unwrap();
        assert_eq!(combos.len(), 1);
        assert!(combos[0].is_empty());
    }

    #[test]
    fn test_cartesian_product_single_dimension() {
        let mut m = HashMap::new();
        m.insert("os".into(), vec!["linux".into(), "macos".into()]);
        let combos = cartesian_product(&m).unwrap();
        assert_eq!(combos.len(), 2);
    }

    #[test]
    fn test_cartesian_product_two_dimensions() {
        let mut m = HashMap::new();
        m.insert("os".into(), vec!["linux".into(), "macos".into()]);
        m.insert("toolchain".into(), vec!["stable".into(), "nightly".into()]);
        let combos = cartesian_product(&m).unwrap();
        assert_eq!(combos.len(), 4);
        // Each combo has both keys
        for c in &combos {
            assert!(c.contains_key("os"));
            assert!(c.contains_key("toolchain"));
        }
    }

    #[test]
    fn test_expand_matrix_job_no_strategy() {
        let job = Job {
            name: Some("Test".into()),
            runs_on: None,
            needs: vec![],
            steps: vec![],
            uses: None,
            with: HashMap::new(),
            strategy: None,
        };
        let expanded = expand_matrix_job("test", &job).unwrap();
        assert_eq!(expanded.len(), 1);
        assert_eq!(expanded[0].display_name, "test");
        assert!(expanded[0].matrix_values.is_empty());
    }

    #[test]
    fn test_expand_matrix_job_with_strategy() {
        let mut matrix = HashMap::new();
        matrix.insert("os".into(), vec!["linux".into(), "macos".into()]);
        matrix.insert("rust".into(), vec!["stable".into(), "nightly".into()]);

        let job = Job {
            name: Some("Test".into()),
            runs_on: None,
            needs: vec![],
            steps: vec![Step {
                name: Some("Run".into()),
                run: Some("cargo test".into()),
                uses: None,
                with: HashMap::new(),
            }],
            uses: None,
            with: HashMap::new(),
            strategy: Some(MatrixStrategy {
                matrix,
                fail_fast: true,
            }),
        };

        let expanded = expand_matrix_job("test", &job).unwrap();
        assert_eq!(expanded.len(), 4);

        // Check display names contain matrix values
        let names: Vec<&str> = expanded.iter().map(|e| e.display_name.as_str()).collect();
        assert!(
            names
                .iter()
                .any(|n: &&str| n.contains("linux") && n.contains("stable"))
        );
        assert!(
            names
                .iter()
                .any(|n: &&str| n.contains("macos") && n.contains("nightly"))
        );

        // All instances should have fail_fast = true
        assert!(expanded.iter().all(|e| e.fail_fast));
    }

    #[test]
    fn test_expand_workflow_matrices() {
        let mut jobs = HashMap::new();

        // Build: no matrix
        jobs.insert(
            "build".to_string(),
            Job {
                name: Some("Build".into()),
                runs_on: None,
                needs: vec![],
                steps: vec![],
                uses: None,
                with: HashMap::new(),
                strategy: None,
            },
        );

        // Test: 2x2 matrix
        let mut matrix = HashMap::new();
        matrix.insert("os".into(), vec!["linux".into(), "macos".into()]);
        matrix.insert("rust".into(), vec!["stable".into(), "nightly".into()]);

        jobs.insert(
            "test".to_string(),
            Job {
                name: Some("Test".into()),
                runs_on: None,
                needs: vec!["build".into()],
                steps: vec![],
                uses: None,
                with: HashMap::new(),
                strategy: Some(MatrixStrategy {
                    matrix,
                    fail_fast: false,
                }),
            },
        );

        let wf = Workflow {
            name: "CI".into(),
            on: vec![],
            jobs,
        };

        let (expanded, order) = expand_workflow_matrices(&wf).unwrap();
        // 1 build + 4 test instances = 5
        assert_eq!(expanded.len(), 5);
        assert_eq!(order.len(), 5);

        // Build should come before all test instances
        let build_pos = order.iter().position(|s| s == "build").unwrap();
        for name in &order {
            if name.starts_with("test") {
                let pos = order.iter().position(|s| s == name).unwrap();
                assert!(build_pos < pos, "build should precede {}", name);
            }
        }

        // Each test instance should depend on build (not original "test")
        for (key, inst) in &expanded {
            if key.starts_with("test") {
                assert!(
                    inst.job.needs.contains(&"build".to_string()),
                    "{} should need build",
                    key
                );
            }
        }
    }
}
