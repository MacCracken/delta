//! Pipeline runner — orchestrates workflow execution end-to-end.
//!
//! Connects the workflow parser, trigger system, executor, and database
//! to run pipelines triggered by repository events.

use crate::executor::{execute_job, resolve_job_order};
use crate::parser::load_workflows;
use crate::trigger::{self, Event};
use delta_core::db;
use sqlx::SqlitePool;
use std::collections::HashMap;
use std::path::Path;

/// Context for running pipelines against a repository.
pub struct PipelineContext<'a> {
    pub pool: &'a SqlitePool,
    pub repo_id: &'a str,
    pub repo_path: &'a Path,
    pub commit_sha: &'a str,
    pub secrets: &'a HashMap<String, String>,
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

        // Resolve job execution order
        let job_order = match resolve_job_order(workflow) {
            Ok(order) => order,
            Err(e) => {
                tracing::error!(workflow = filename, "invalid job graph: {}", e);
                let _ = db::pipeline::update_pipeline_status(
                    ctx.pool,
                    &pipeline.id,
                    db::pipeline::RunStatus::Failed,
                )
                .await;
                continue;
            }
        };

        // Mark pipeline as running
        let _ = db::pipeline::update_pipeline_status(
            ctx.pool,
            &pipeline.id,
            db::pipeline::RunStatus::Running,
        )
        .await;

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
            let Some(job) = workflow.jobs.get(job_name) else {
                continue;
            };

            // Create job record
            let job_run = match db::pipeline::create_job(ctx.pool, &pipeline.id, job_name).await {
                Ok(j) => j,
                Err(e) => {
                    tracing::error!(job = job_name, "failed to create job record: {}", e);
                    pipeline_passed = false;
                    break;
                }
            };

            // Mark job as running
            let _ = db::pipeline::update_job_status(
                ctx.pool,
                &job_run.id,
                db::pipeline::RunStatus::Running,
                None,
            )
            .await;

            // Execute the job
            let result = execute_job(job_name, job, ctx.repo_path, &env_vars).await;

            // Store step logs (mask secret values)
            for (idx, step) in result.steps.iter().enumerate() {
                let mut output = format!("{}{}", step.stdout, step.stderr);
                for (_, secret_value) in ctx.secrets.iter() {
                    if !secret_value.is_empty() {
                        output = output.replace(secret_value, "***");
                    }
                }
                let status = if step.exit_code == 0 {
                    "passed"
                } else {
                    "failed"
                };
                let _ = db::pipeline::append_step_log(
                    ctx.pool,
                    &job_run.id,
                    &step.name,
                    idx as i64,
                    &output,
                    status,
                )
                .await;
            }

            // Update job status
            let (status, exit_code) = if result.success {
                (db::pipeline::RunStatus::Passed, Some(0))
            } else {
                let code = result.steps.last().map(|s| s.exit_code).unwrap_or(-1);
                (db::pipeline::RunStatus::Failed, Some(code))
            };
            let _ = db::pipeline::update_job_status(ctx.pool, &job_run.id, status, exit_code).await;

            if !result.success {
                pipeline_passed = false;
                break;
            }
        }

        // Update pipeline status
        let final_status = if pipeline_passed {
            db::pipeline::RunStatus::Passed
        } else {
            db::pipeline::RunStatus::Failed
        };
        let _ = db::pipeline::update_pipeline_status(ctx.pool, &pipeline.id, final_status).await;

        tracing::info!(
            workflow = filename,
            pipeline_id = %pipeline.id,
            status = ?final_status,
            "pipeline complete"
        );
    }
}
