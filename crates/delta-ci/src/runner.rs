//! Pipeline runner — orchestrates workflow execution end-to-end.
//!
//! Connects the workflow parser, trigger system, executor, and database
//! to run pipelines triggered by repository events. Jobs with
//! `runs_on: "self-hosted, ..."` are queued for remote runners instead
//! of being executed locally.

use crate::events::{PipelineEvent, PipelineStreams};
use crate::executor::{SandboxMode, execute_job, expand_workflow_matrices};
use crate::parser::load_workflows;
use crate::remote;
use crate::trigger::{self, Event};
use delta_core::db;
use sqlx::SqlitePool;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use tokio::sync::broadcast;

/// Context for running pipelines against a repository.
pub struct PipelineContext<'a> {
    pub pool: &'a SqlitePool,
    pub repo_id: &'a str,
    /// Working tree of the commit being built: workflows are read from it and
    /// steps run in it. Must be a checkout, never the hosted bare repository.
    pub repo_path: &'a Path,
    /// `$HOME` for locally executed steps.
    pub home_dir: Option<&'a Path>,
    pub commit_sha: &'a str,
    pub secrets: &'a HashMap<String, String>,
    pub streams: Option<&'a PipelineStreams>,
    pub sandbox: SandboxMode,
    /// Whether self-hosted runners are enabled (ci.runner_token is set).
    pub runners_enabled: bool,
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

/// Run all matching workflows for a pushed tag.
pub async fn run_tag_pipelines(ctx: &PipelineContext<'_>, tag: &str) {
    let event = Event::Tag {
        tag_name: tag.to_string(),
    };
    run_pipelines(ctx, &event, "tag", Some(tag)).await;
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
        execute_pipeline(
            ctx,
            filename,
            workflow,
            &pipeline.id,
            trigger_type,
            trigger_ref,
        )
        .await;
    }
}

/// Run an existing (queued) pipeline record, e.g. one created by a manual
/// trigger, with the repository's workflow named `workflow_name` (its `name`
/// or file name). The pipeline fails if there is no such workflow.
pub async fn run_workflow(
    ctx: &PipelineContext<'_>,
    pipeline_id: &str,
    workflow_name: &str,
    trigger_type: &str,
    trigger_ref: Option<&str>,
) {
    let workflows = load_workflows(ctx.repo_path);
    let Some((filename, workflow)) = workflows
        .iter()
        .find(|(file, wf)| wf.name == workflow_name || file == workflow_name)
    else {
        tracing::warn!(pipeline_id, workflow = workflow_name, "workflow not found");
        if let Err(e) = db::pipeline::update_pipeline_status(
            ctx.pool,
            pipeline_id,
            db::pipeline::RunStatus::Failed,
        )
        .await
        {
            tracing::error!(pipeline_id, "failed to mark pipeline as failed: {}", e);
        }
        return;
    };
    execute_pipeline(
        ctx,
        filename,
        workflow,
        pipeline_id,
        trigger_type,
        trigger_ref,
    )
    .await;
}

/// Execute `workflow` as the pipeline `pipeline_id`: run its jobs, record
/// their logs and statuses, and finalize the pipeline.
async fn execute_pipeline(
    ctx: &PipelineContext<'_>,
    filename: &str,
    workflow: &crate::workflow::Workflow,
    pipeline_id: &str,
    trigger_type: &str,
    trigger_ref: Option<&str>,
) {
    let pipeline = match db::pipeline::get_pipeline(ctx.pool, pipeline_id).await {
        // Cancelled before it started.
        Ok(run) if run.status == db::pipeline::RunStatus::Cancelled => return,
        Ok(run) => run,
        Err(e) => {
            tracing::error!(pipeline_id, "failed to load pipeline: {}", e);
            return;
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
            return;
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
    if let Some(home) = ctx.home_dir {
        env_vars.insert("HOME".into(), home.display().to_string());
    }

    let secret_needles = crate::mask::secret_needles(ctx.secrets.values().map(String::as_str));

    let mut pipeline_passed = true;
    let mut cancelled = false;
    // Original job keys with a failed or skipped instance (dependents of
    // these must not run) and keys dispatched to self-hosted runners
    // (they finish asynchronously, so nothing here can wait for them).
    let mut failed_keys: HashSet<String> = HashSet::new();
    let mut remote_keys: HashSet<String> = HashSet::new();
    // Matrix groups whose fail_fast was triggered.
    let mut failed_fast_keys: HashSet<String> = HashSet::new();

    for job_name in &job_order {
        let Some(expanded) = expanded_jobs.get(job_name) else {
            continue;
        };

        // Stop starting jobs once the pipeline has been cancelled.
        if matches!(
            db::pipeline::get_pipeline(ctx.pool, &pipeline.id).await,
            Ok(run) if run.status == db::pipeline::RunStatus::Cancelled
        ) {
            cancelled = true;
            break;
        }

        if failed_fast_keys.contains(&expanded.key) {
            // fail_fast: remaining instances of this matrix group don't run.
            failed_keys.insert(expanded.key.clone());
            continue;
        }

        // A job runs only after everything it needs has passed.
        let blocked_reason = if let Some(dep) =
            expanded.job.needs.iter().find(|n| failed_keys.contains(*n))
        {
            Some(format!("skipped: needed job '{dep}' did not succeed"))
        } else {
            expanded
                .job
                .needs
                .iter()
                .find(|n| remote_keys.contains(*n))
                .map(|dep| {
                    format!("jobs cannot depend on self-hosted job '{dep}': it runs asynchronously")
                })
        };

        // Create job record with display name (includes matrix values)
        let job_run =
            match db::pipeline::create_job(ctx.pool, &pipeline.id, &expanded.display_name).await {
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

        if let Some(reason) = blocked_reason {
            tracing::info!(job = &expanded.display_name, "{}", reason);
            if let Err(e) = db::pipeline::append_step_log(
                ctx.pool,
                &job_run.id,
                "dependencies",
                0,
                &reason,
                "failed",
            )
            .await
            {
                tracing::error!(job_id = %job_run.id, "failed to store step log: {}", e);
            }
            if let Err(e) = db::pipeline::update_job_status(
                ctx.pool,
                &job_run.id,
                db::pipeline::RunStatus::Failed,
                None,
            )
            .await
            {
                tracing::error!(job_id = %job_run.id, "failed to mark job as failed: {}", e);
            }
            let _ = tx.send(PipelineEvent::JobCompleted {
                job_id: job_run.id.clone(),
                success: false,
                exit_code: None,
            });
            failed_keys.insert(expanded.key.clone());
            pipeline_passed = false;
            continue;
        }

        // Inject MATRIX_* env vars for this instance
        let mut job_env = env_vars.clone();
        for (dim, val) in &expanded.matrix_values {
            job_env.insert(format!("MATRIX_{}", dim.to_uppercase()), val.clone());
        }

        // Check if this job targets a self-hosted runner
        if let Some(labels) = ctx
            .runners_enabled
            .then(|| remote::parse_self_hosted(expanded.job.runs_on.as_deref()))
            .flatten()
        {
            // Enqueue for remote execution — runner will pick it up via poll.
            // Strip secrets from the env — runners should not receive repo secrets.
            // Only DELTA_* and MATRIX_* vars are included.
            let safe_env: HashMap<String, String> = job_env
                .iter()
                .filter(|(k, _)| k.starts_with("DELTA_") || k.starts_with("MATRIX_"))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();

            // Pre-generate queue ID so the payload includes it directly,
            // eliminating the two-phase update.
            let queue_id = uuid::Uuid::new_v4().to_string();

            let payload = remote::build_payload(
                &queue_id,
                &job_run.id,
                &pipeline.id,
                ctx.repo_id,
                &expanded.display_name,
                &expanded.job,
                &safe_env,
                ctx.commit_sha,
            );
            let payload_json = match serde_json::to_string(&payload) {
                Ok(j) => j,
                Err(e) => {
                    tracing::error!(
                        job = &expanded.display_name,
                        "failed to serialize job payload: {}",
                        e
                    );
                    pipeline_passed = false;
                    break;
                }
            };

            match db::runner::enqueue_job(
                ctx.pool,
                &queue_id,
                &job_run.id,
                &pipeline.id,
                ctx.repo_id,
                &labels,
                &payload_json,
            )
            .await
            {
                Ok(queued) => {
                    tracing::info!(
                        job = &expanded.display_name,
                        queue_id = %queued.id,
                        labels = ?labels,
                        "job queued for self-hosted runner"
                    );
                }
                Err(e) => {
                    tracing::error!(job = &expanded.display_name, "failed to enqueue job: {}", e);
                    if let Err(e) = db::pipeline::update_job_status(
                        ctx.pool,
                        &job_run.id,
                        db::pipeline::RunStatus::Failed,
                        Some(-1),
                    )
                    .await
                    {
                        tracing::error!(job_id = %job_run.id, "failed to mark job as failed: {}", e);
                    }
                    pipeline_passed = false;
                    failed_keys.insert(expanded.key.clone());
                    if expanded.fail_fast {
                        failed_fast_keys.insert(expanded.key.clone());
                    }
                }
            }
            // Don't wait for remote jobs — they complete asynchronously
            remote_keys.insert(expanded.key.clone());
            continue;
        }

        // --- Local execution path ---

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
            &secret_needles,
        )
        .await;

        // Store step logs (mask secret values)
        for (idx, step) in result.steps.iter().enumerate() {
            // Lines were masked as they streamed; mask the joined output
            // again to catch multi-line secrets.
            let output = crate::mask::mask_secrets(
                &format!("{}{}", step.stdout, step.stderr),
                &secret_needles,
            );
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
            failed_keys.insert(expanded.key.clone());
            if expanded.fail_fast {
                tracing::info!(
                    job = &expanded.display_name,
                    "fail_fast: stopping remaining matrix jobs"
                );
                failed_fast_keys.insert(expanded.key.clone());
            }
        }
    }

    if cancelled {
        tracing::info!(pipeline_id = %pipeline.id, "pipeline cancelled; not starting further jobs");
        let _ = tx.send(PipelineEvent::PipelineCompleted {
            status: "cancelled".to_string(),
        });
        if let Some(streams) = ctx.streams {
            streams.remove(&pipeline.id);
        }
        return;
    }

    // Check if any jobs were dispatched to remote runners (still queued).
    // If so, don't finalize the pipeline yet — the runner completion handler
    // will finalize when all jobs are done.
    let jobs = db::pipeline::list_jobs(ctx.pool, &pipeline.id)
        .await
        .unwrap_or_default();
    let has_pending_remote = jobs.iter().any(|j| {
        matches!(
            j.status,
            db::pipeline::RunStatus::Queued | db::pipeline::RunStatus::Running
        )
    });

    if has_pending_remote {
        tracing::info!(
            pipeline_id = %pipeline.id,
            "pipeline has remote jobs still pending — deferring finalization"
        );
    } else {
        // All jobs completed locally — finalize now
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
    }

    tracing::info!(
        workflow = filename,
        pipeline_id = %pipeline.id,
        passed = pipeline_passed,
        has_remote = has_pending_remote,
        "pipeline local jobs complete"
    );
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
