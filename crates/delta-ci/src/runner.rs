//! Pipeline runner — orchestrates workflow execution end-to-end.
//!
//! Connects the workflow parser, trigger system, executor, and database
//! to run pipelines triggered by repository events.

use crate::events::{PipelineEvent, PipelineStreams};
use crate::executor::{SandboxMode, execute_job, expand_workflow_matrices};
use crate::parser::load_workflows;
use crate::trigger::{self, Event};
use delta_core::db;
use sqlx::SqlitePool;
use std::collections::HashMap;
use std::path::Path;
use tokio::sync::broadcast;

/// Context for running pipelines against a repository.
pub struct PipelineContext<'a> {
    pub pool: &'a SqlitePool,
    pub repo_id: &'a str,
    pub repo_path: &'a Path,
    pub commit_sha: &'a str,
    pub secrets: &'a HashMap<String, String>,
    pub streams: Option<&'a PipelineStreams>,
    pub sandbox: SandboxMode,
}

/// Run all matching workflows for a push event.
///
/// This is the main entry point for event-driven pipeline execution.
/// It loads workflows from the repo, checks triggers, creates DB records,
/// executes jobs, captures logs, and updates statuses.
pub async fn run_push_pipelines(ctx: &PipelineContext<'_>, branch: &str) {
    let event = Event::Push {
        branch: branch.to_string(),
    };
    run_pipelines(ctx, &event, "push", Some(branch)).await;
}

/// Run pipelines for any event type.
async fn run_pipelines(
    ctx: &PipelineContext<'_>,
    event: &Event,
    trigger_type: &str,
    trigger_ref: Option<&str>,
) {
    let workflows = load_workflows(ctx.repo_path);
    if workflows.is_empty() {
        return;
    }

    for (filename, workflow) in &workflows {
        if !trigger::should_trigger(workflow, event) {
            continue;
        }

        tracing::info!(
            workflow = filename,
            trigger = trigger_type,
            "triggering pipeline"
        );

        let pipeline = match db::pipeline::create_pipeline(
            ctx.pool,
            ctx.repo_id,
            &workflow.name,
            trigger_type,
            trigger_ref,
            ctx.commit_sha,
        )
        .await
        {
            Ok(p) => p,
            Err(e) => {
                tracing::error!(workflow = filename, "failed to create pipeline: {}", e);
                continue;
            }
        };

        // Set up broadcast channel for this pipeline
        let (tx, _) = broadcast::channel::<PipelineEvent>(256);
        if let Some(streams) = ctx.streams {
            streams.insert(pipeline.id.clone(), tx.clone());
        }

        // Expand matrix jobs and resolve execution order
        let (expanded_jobs, job_order) = match expand_workflow_matrices(workflow) {
            Ok(result) => result,
            Err(e) => {
                tracing::error!(workflow = filename, "invalid job graph: {}", e);
                if let Err(e) = db::pipeline::update_pipeline_status(
                    ctx.pool,
                    &pipeline.id,
                    db::pipeline::RunStatus::Failed,
                )
                .await
                {
                    tracing::error!(pipeline_id = %pipeline.id, "failed to mark pipeline as failed: {}", e);
                }
                let _ = tx.send(PipelineEvent::PipelineCompleted {
                    status: "failed".to_string(),
                });
                if let Some(streams) = ctx.streams {
                    streams.remove(&pipeline.id);
                }
                continue;
            }
        };

        // Mark pipeline as running
        if let Err(e) = db::pipeline::update_pipeline_status(
            ctx.pool,
            &pipeline.id,
            db::pipeline::RunStatus::Running,
        )
        .await
        {
            tracing::error!(pipeline_id = %pipeline.id, "failed to mark pipeline as running: {}", e);
        }

        // Build environment variables for jobs
        let mut env_vars = ctx.secrets.clone();
        env_vars.insert("DELTA_PIPELINE_ID".into(), pipeline.id.clone());
        env_vars.insert("DELTA_REPO_ID".into(), ctx.repo_id.to_string());
        env_vars.insert("DELTA_COMMIT_SHA".into(), ctx.commit_sha.to_string());
        env_vars.insert("DELTA_TRIGGER".into(), trigger_type.to_string());
        if let Some(r) = trigger_ref {
            env_vars.insert("DELTA_REF".into(), r.to_string());
        }

        let mut pipeline_passed = true;

        for job_name in &job_order {
            let Some(expanded) = expanded_jobs.get(job_name) else {
                continue;
            };

            // Create job record with display name (includes matrix values)
            let job_run = match db::pipeline::create_job(
                ctx.pool,
                &pipeline.id,
                &expanded.display_name,
            )
            .await
            {
                Ok(j) => j,
                Err(e) => {
                    tracing::error!(job = job_name, "failed to create job record: {}", e);
                    pipeline_passed = false;
                    break;
                }
            };

            // Emit job started event
            let _ = tx.send(PipelineEvent::JobStarted {
                job_name: expanded.display_name.clone(),
                job_id: job_run.id.clone(),
            });

            // Mark job as running
            if let Err(e) = db::pipeline::update_job_status(
                ctx.pool,
                &job_run.id,
                db::pipeline::RunStatus::Running,
                None,
            )
            .await
            {
                tracing::error!(job_id = %job_run.id, "failed to mark job as running: {}", e);
            }

            // Inject MATRIX_* env vars for this instance
            let mut job_env = env_vars.clone();
            for (dim, val) in &expanded.matrix_values {
                job_env.insert(format!("MATRIX_{}", dim.to_uppercase()), val.clone());
            }

            // Determine sandbox mode for this job
            let job_sandbox = resolve_job_sandbox(&ctx.sandbox, expanded.job.runs_on.as_deref());

            // Execute the job with streaming
            let result = execute_job(
                &expanded.display_name,
                &expanded.job,
                ctx.repo_path,
                &job_env,
                Some(&job_run.id),
                Some(&tx),
                &job_sandbox,
            )
            .await;

            // Store step logs (mask secret values)
            for (idx, step) in result.steps.iter().enumerate() {
                let mut output = format!("{}{}", step.stdout, step.stderr);
                for (_, secret_value) in ctx.secrets.iter() {
                    if !secret_value.is_empty() {
                        // Case-insensitive masking (char-safe for UTF-8)
                        let lower_secret = secret_value.to_lowercase();
                        let secret_chars: Vec<char> = lower_secret.chars().collect();
                        let output_chars: Vec<char> = output.chars().collect();
                        let lower_chars: Vec<char> = output.to_lowercase().chars().collect();
                        let mut masked = String::with_capacity(output.len());
                        let mut i = 0;
                        while i < output_chars.len() {
                            if i + secret_chars.len() <= lower_chars.len()
                                && lower_chars[i..i + secret_chars.len()] == secret_chars[..]
                            {
                                masked.push_str("***");
                                i += secret_chars.len();
                            } else {
                                masked.push(output_chars[i]);
                                i += 1;
                            }
                        }
                        output = masked;
                        // Also mask URL-encoded form
                        let url_encoded: String = secret_value
                            .bytes()
                            .map(|b| {
                                if b.is_ascii_alphanumeric()
                                    || b == b'-'
                                    || b == b'_'
                                    || b == b'.'
                                    || b == b'~'
                                {
                                    (b as char).to_string()
                                } else {
                                    format!("%{:02X}", b)
                                }
                            })
                            .collect();
                        if url_encoded != *secret_value {
                            output = output.replace(&url_encoded, "***");
                        }
                    }
                }
                let status = if step.exit_code == 0 {
                    "passed"
                } else {
                    "failed"
                };
                if let Err(e) = db::pipeline::append_step_log(
                    ctx.pool,
                    &job_run.id,
                    &step.name,
                    idx as i64,
                    &output,
                    status,
                )
                .await
                {
                    tracing::error!(job_id = %job_run.id, step = &step.name, "failed to store step log: {}", e);
                }
            }

            // Update job status
            let (status, exit_code) = if result.success {
                (db::pipeline::RunStatus::Passed, Some(0))
            } else {
                let code = result.steps.last().map(|s| s.exit_code).unwrap_or(-1);
                (db::pipeline::RunStatus::Failed, Some(code))
            };

            // Emit job completed event
            let _ = tx.send(PipelineEvent::JobCompleted {
                job_id: job_run.id.clone(),
                success: result.success,
                exit_code,
            });

            if let Err(e) =
                db::pipeline::update_job_status(ctx.pool, &job_run.id, status, exit_code).await
            {
                tracing::error!(job_id = %job_run.id, "failed to update job status: {}", e);
            }

            if !result.success {
                pipeline_passed = false;
                if expanded.fail_fast {
                    tracing::info!(
                        job = &expanded.display_name,
                        "fail_fast: stopping remaining matrix jobs"
                    );
                    break;
                }
            }
        }

        // Update pipeline status
        let final_status = if pipeline_passed {
            db::pipeline::RunStatus::Passed
        } else {
            db::pipeline::RunStatus::Failed
        };

        // Emit pipeline completed event
        let _ = tx.send(PipelineEvent::PipelineCompleted {
            status: format!("{:?}", final_status).to_lowercase(),
        });

        // Remove broadcast channel from registry
        if let Some(streams) = ctx.streams {
            streams.remove(&pipeline.id);
        }

        if let Err(e) =
            db::pipeline::update_pipeline_status(ctx.pool, &pipeline.id, final_status).await
        {
            tracing::error!(pipeline_id = %pipeline.id, "failed to update pipeline final status: {}", e);
        }

        tracing::info!(
            workflow = filename,
            pipeline_id = %pipeline.id,
            status = ?final_status,
            "pipeline complete"
        );
    }
}

/// Resolve the sandbox mode for a specific job, considering the runs_on field.
fn resolve_job_sandbox(base: &SandboxMode, runs_on: Option<&str>) -> SandboxMode {
    // If runs_on specifies a container image, use container mode
    if let Some(runs_on) = runs_on
        && let Some(image) = runs_on.strip_prefix("docker://")
    {
        if let Some(runtime) = crate::container::detect_runtime() {
            return SandboxMode::Container {
                runtime,
                image: image.to_string(),
            };
        }
        tracing::warn!(
            "runs_on specifies container image '{}' but no container runtime found",
            runs_on
        );
    }
    base.clone()
}
