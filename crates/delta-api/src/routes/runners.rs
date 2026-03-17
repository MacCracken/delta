//! Self-hosted CI runner API routes.
//!
//! Runners authenticate using the shared `ci.runner_token` from config plus
//! their individual runner name/token pair. Admin users can list and remove runners.

use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    routing::{delete, get, post},
};
use delta_core::db;
use serde::{Deserialize, Serialize};

use crate::extractors::AuthUser;
use crate::state::AppState;

/// Maximum size of a single step log output (1 MB).
const MAX_STEP_LOG_SIZE: usize = 1_048_576;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/register", post(register_runner))
        .route("/list", get(list_runners))
        .route("/{runner_id}", delete(remove_runner))
        .route("/poll", post(poll_job))
        .route("/heartbeat", post(runner_heartbeat))
        .route("/jobs/{job_id}/start", post(start_job))
        .route("/jobs/{job_id}/log", post(submit_step_log))
        .route("/jobs/{job_id}/complete", post(complete_job))
}

// --- Runner authentication helper ---

/// Authenticate a runner request via the `X-Runner-Name` and `X-Runner-Token` headers,
/// validated against the shared `ci.runner_token` in config and the runner's stored token hash.
async fn authenticate_runner(
    state: &AppState,
    headers: &axum::http::HeaderMap,
) -> Result<db::runner::Runner, (StatusCode, String)> {
    let runner_token = state.config.ci.runner_token.as_deref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "runner support not configured (set ci.runner_token)".into(),
    ))?;

    let name = headers
        .get("x-runner-name")
        .and_then(|v| v.to_str().ok())
        .ok_or((StatusCode::UNAUTHORIZED, "missing X-Runner-Name header".into()))?;

    let token = headers
        .get("x-runner-token")
        .and_then(|v| v.to_str().ok())
        .ok_or((StatusCode::UNAUTHORIZED, "missing X-Runner-Token header".into()))?;

    // Validate shared runner token via hash comparison (constant-time, no length leak).
    // The runner's full token must equal the configured runner_token exactly.
    let expected_hash = hash_token(runner_token);
    let provided_hash = hash_token(token);
    if !constant_time_eq(expected_hash.as_bytes(), provided_hash.as_bytes()) {
        return Err((StatusCode::UNAUTHORIZED, "invalid runner token".into()));
    }

    let token_hash = provided_hash;
    let runner = db::runner::authenticate(&state.db, name, &token_hash)
        .await
        .map_err(|_| (StatusCode::UNAUTHORIZED, "invalid runner credentials".into()))?;

    Ok(runner)
}

fn hash_token(token: &str) -> String {
    blake3::hash(token.as_bytes()).to_hex().to_string()
}

/// Constant-time byte comparison to prevent timing attacks.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Verify that a runner has claimed a specific job via the queue.
async fn verify_runner_owns_job(
    pool: &sqlx::SqlitePool,
    runner_id: &str,
    job_id: &str,
) -> Result<db::runner::QueuedJob, (StatusCode, String)> {
    let queued = db::runner::get_queued_job_by_job_run_id(pool, job_id)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "job not found in runner queue".into()))?;

    if queued.claimed_by.as_deref() != Some(runner_id) {
        return Err((
            StatusCode::FORBIDDEN,
            "this runner has not claimed this job".into(),
        ));
    }

    Ok(queued)
}

// --- Registration ---

#[derive(Deserialize)]
struct RegisterRequest {
    name: String,
    token: String,
    #[serde(default)]
    labels: Vec<String>,
}

#[derive(Serialize)]
struct RunnerResponse {
    id: String,
    name: String,
    labels: Vec<String>,
    status: String,
    last_heartbeat_at: Option<String>,
    created_at: String,
}

impl From<db::runner::Runner> for RunnerResponse {
    fn from(r: db::runner::Runner) -> Self {
        Self {
            id: r.id,
            name: r.name,
            labels: r.labels,
            status: r.status.as_str().to_string(),
            last_heartbeat_at: r.last_heartbeat_at,
            created_at: r.created_at,
        }
    }
}

async fn register_runner(
    State(state): State<AppState>,
    Json(req): Json<RegisterRequest>,
) -> Result<(StatusCode, Json<RunnerResponse>), (StatusCode, String)> {
    let runner_token = state.config.ci.runner_token.as_deref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "runner support not configured (set ci.runner_token)".into(),
    ))?;

    // Validate the shared runner token via hash comparison (constant-time)
    let expected_hash = hash_token(runner_token);
    let provided_hash = hash_token(&req.token);
    if !constant_time_eq(expected_hash.as_bytes(), provided_hash.as_bytes()) {
        return Err((StatusCode::UNAUTHORIZED, "invalid runner token".into()));
    }

    // Validate runner name
    if req.name.is_empty()
        || req.name.len() > 128
        || !req
            .name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "runner name must be 1-128 alphanumeric characters, hyphens, underscores, or dots"
                .into(),
        ));
    }

    // Validate labels: no commas, max 64 chars each, max 16 labels
    if req.labels.len() > 16 {
        return Err((
            StatusCode::BAD_REQUEST,
            "maximum 16 labels per runner".into(),
        ));
    }
    for label in &req.labels {
        if label.is_empty()
            || label.len() > 64
            || label.contains(',')
            || !label
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
        {
            return Err((
                StatusCode::BAD_REQUEST,
                "labels must be 1-64 alphanumeric characters, hyphens, underscores, or dots"
                    .into(),
            ));
        }
    }

    let runner = db::runner::register(&state.db, &req.name, &provided_hash, &req.labels)
        .await
        .map_err(|e| {
            tracing::error!("failed to register runner: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "failed to register runner".into(),
            )
        })?;

    tracing::info!(runner_name = %runner.name, runner_id = %runner.id, "runner registered");

    Ok((StatusCode::CREATED, Json(runner.into())))
}

// --- List runners (admin only — requires first user / site admin) ---

async fn list_runners(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
) -> Result<Json<Vec<RunnerResponse>>, (StatusCode, String)> {
    crate::helpers::require_site_admin(&state, &user).await?;

    let runners = db::runner::list(&state.db).await.map_err(|e| {
        tracing::error!("failed to list runners: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal server error".into(),
        )
    })?;

    Ok(Json(runners.into_iter().map(RunnerResponse::from).collect()))
}

// --- Remove runner (admin only) ---

async fn remove_runner(
    State(state): State<AppState>,
    axum::extract::Path(runner_id): axum::extract::Path<String>,
    AuthUser(user): AuthUser,
) -> Result<StatusCode, (StatusCode, String)> {
    crate::helpers::require_site_admin(&state, &user).await?;

    db::runner::delete(&state.db, &runner_id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;

    tracing::info!(runner_id = %runner_id, "runner removed");
    Ok(StatusCode::NO_CONTENT)
}

// --- Poll for jobs ---

#[derive(Serialize)]
struct PollResponse {
    job: Option<delta_ci::remote::JobPayload>,
}

async fn poll_job(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
) -> Result<Json<PollResponse>, (StatusCode, String)> {
    let runner = authenticate_runner(&state, &headers).await?;

    // Update heartbeat
    let _ = db::runner::heartbeat(&state.db, &runner.id).await;

    let queued = db::runner::poll_job(&state.db, &runner.id, &runner.labels)
        .await
        .map_err(|e| {
            tracing::error!("failed to poll jobs: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal server error".into(),
            )
        })?;

    let job = match queued {
        Some(q) => {
            let payload: delta_ci::remote::JobPayload =
                serde_json::from_str(&q.payload).map_err(|e| {
                    tracing::error!("corrupt job payload: {}", e);
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "corrupt job payload".into(),
                    )
                })?;
            Some(payload)
        }
        None => None,
    };

    Ok(Json(PollResponse { job }))
}

// --- Heartbeat ---

async fn runner_heartbeat(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
) -> Result<StatusCode, (StatusCode, String)> {
    let runner = authenticate_runner(&state, &headers).await?;
    db::runner::heartbeat(&state.db, &runner.id)
        .await
        .map_err(|e| {
            tracing::error!("failed to update heartbeat: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal server error".into(),
            )
        })?;
    Ok(StatusCode::NO_CONTENT)
}

// --- Start job (runner marks it as running) ---

async fn start_job(
    State(state): State<AppState>,
    axum::extract::Path(job_id): axum::extract::Path<String>,
    headers: axum::http::HeaderMap,
) -> Result<StatusCode, (StatusCode, String)> {
    let runner = authenticate_runner(&state, &headers).await?;

    // Verify this runner has claimed this job
    let _queued = verify_runner_owns_job(&state.db, &runner.id, &job_id).await?;

    // Mark the job_run as running
    db::pipeline::update_job_status(
        &state.db,
        &job_id,
        db::pipeline::RunStatus::Running,
        None,
    )
    .await
    .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;

    // Update the runner field on the job
    db::runner::set_job_runner(&state.db, &job_id, &runner.name)
        .await
        .map_err(|e| {
            tracing::error!("failed to set runner on job: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal server error".into(),
            )
        })?;

    // Emit event to pipeline stream — use actual job name from the DB
    if let Ok(job) = db::pipeline::get_job(&state.db, &job_id).await
        && let Some(sender) = state.pipeline_streams.get(&job.pipeline_id)
    {
        let _ = sender.send(delta_ci::PipelineEvent::JobStarted {
            job_name: job.job_name,
            job_id: job_id.clone(),
        });
    }

    Ok(StatusCode::NO_CONTENT)
}

// --- Submit step log ---

#[derive(Deserialize)]
struct StepLogRequest {
    step_name: String,
    step_index: i64,
    output: String,
    status: String,
}

async fn submit_step_log(
    State(state): State<AppState>,
    axum::extract::Path(job_id): axum::extract::Path<String>,
    headers: axum::http::HeaderMap,
    Json(req): Json<StepLogRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    let runner = authenticate_runner(&state, &headers).await?;

    // Validate input
    if req.step_name.len() > 256 {
        return Err((StatusCode::BAD_REQUEST, "step_name too long (max 256)".into()));
    }
    if req.step_index < 0 || req.step_index > 1000 {
        return Err((StatusCode::BAD_REQUEST, "step_index must be 0-1000".into()));
    }

    // Verify this runner has claimed this job
    let queued = verify_runner_owns_job(&state.db, &runner.id, &job_id).await?;

    // Enforce output size limit
    let output = truncate_output(&req.output);

    // Mask secrets in the output
    let masked_output = mask_secrets_for_job(&state, &queued.repo_id, &output).await;

    db::pipeline::append_step_log(
        &state.db,
        &job_id,
        &req.step_name,
        req.step_index,
        &masked_output,
        &req.status,
    )
    .await
    .map_err(|e| {
        tracing::error!("failed to append step log: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal server error".into(),
        )
    })?;

    // Emit step output event to pipeline stream
    if let Some(pipeline_id) = get_pipeline_id_for_job(&state.db, &job_id).await
        && let Some(sender) = state.pipeline_streams.get(&pipeline_id)
    {
        let _ = sender.send(delta_ci::PipelineEvent::StepCompleted {
            job_id: job_id.clone(),
            step_index: req.step_index as usize,
            exit_code: if req.status == "passed" { 0 } else { 1 },
        });
    }

    Ok(StatusCode::NO_CONTENT)
}

// --- Complete job ---

#[derive(Deserialize)]
struct CompleteJobRequest {
    queue_id: String,
    success: bool,
    steps: Vec<StepReportRequest>,
}

#[derive(Deserialize)]
struct StepReportRequest {
    name: String,
    exit_code: i32,
    output: String,
}

async fn complete_job(
    State(state): State<AppState>,
    axum::extract::Path(job_id): axum::extract::Path<String>,
    headers: axum::http::HeaderMap,
    Json(req): Json<CompleteJobRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    let runner = authenticate_runner(&state, &headers).await?;

    // Verify this runner has claimed this job AND the queue_id matches
    let queued = verify_runner_owns_job(&state.db, &runner.id, &job_id).await?;
    if queued.id != req.queue_id {
        return Err((
            StatusCode::FORBIDDEN,
            "queue_id does not match this job".into(),
        ));
    }

    // Store step logs from the runner with secret masking
    for (idx, step) in req.steps.iter().enumerate() {
        let status = if step.exit_code == 0 {
            "passed"
        } else {
            "failed"
        };
        let output = truncate_output(&step.output);
        let masked_output = mask_secrets_for_job(&state, &queued.repo_id, &output).await;
        let _ = db::pipeline::append_step_log(
            &state.db,
            &job_id,
            &step.name,
            idx as i64,
            &masked_output,
            status,
        )
        .await;
    }

    // Update job status
    let (status, exit_code) = if req.success {
        (db::pipeline::RunStatus::Passed, Some(0))
    } else {
        let code = req.steps.last().map(|s| s.exit_code).unwrap_or(-1);
        (db::pipeline::RunStatus::Failed, Some(code))
    };

    db::pipeline::update_job_status(&state.db, &job_id, status, exit_code)
        .await
        .map_err(|e| {
            tracing::error!("failed to update job status: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal server error".into(),
            )
        })?;

    // Mark queued job as complete (with status guard)
    let _ = db::runner::complete_queued_job(&state.db, &req.queue_id, &runner.id).await;

    // Emit job completed event to pipeline stream
    if let Some(pipeline_id) = get_pipeline_id_for_job(&state.db, &job_id).await {
        if let Some(sender) = state.pipeline_streams.get(&pipeline_id) {
            let _ = sender.send(delta_ci::PipelineEvent::JobCompleted {
                job_id: job_id.clone(),
                success: req.success,
                exit_code,
            });
        }

        // Check if all jobs in the pipeline are done; if so, finalize the pipeline
        finalize_pipeline_if_done(&state.db, &pipeline_id, &state.pipeline_streams).await;
    }

    Ok(StatusCode::NO_CONTENT)
}

// --- Helpers ---

/// Truncate output to MAX_STEP_LOG_SIZE, respecting UTF-8 char boundaries.
fn truncate_output(s: &str) -> String {
    if s.len() <= MAX_STEP_LOG_SIZE {
        return s.to_string();
    }
    // Find the last char boundary at or before the limit
    let truncated = match s.get(..MAX_STEP_LOG_SIZE) {
        Some(valid) => valid,
        None => {
            // Binary search for last valid char boundary
            let mut end = MAX_STEP_LOG_SIZE;
            while end > 0 && !s.is_char_boundary(end) {
                end -= 1;
            }
            &s[..end]
        }
    };
    format!("{}\n... [output truncated at 1 MB]", truncated)
}

async fn get_pipeline_id_for_job(pool: &sqlx::SqlitePool, job_id: &str) -> Option<String> {
    let job = db::pipeline::get_job(pool, job_id).await.ok()?;
    Some(job.pipeline_id)
}

/// Mask secret values in output text for a given repo.
async fn mask_secrets_for_job(state: &AppState, repo_id: &str, output: &str) -> String {
    let encryption_key = delta_core::crypto::derive_key(&state.config.auth.secrets_key);
    let secrets = match db::secret::get_all_values(&state.db, repo_id).await {
        Ok(s) => s,
        Err(_) => return output.to_string(),
    };

    let mut masked = output.to_string();
    for (_, encrypted_value) in &secrets {
        if let Ok(value) = delta_core::crypto::decrypt(&encryption_key, encrypted_value)
            && !value.is_empty()
        {
            masked = masked.replace(&value, "***");
        }
    }
    masked
}

/// Check if all jobs in a pipeline are terminal (passed/failed/cancelled).
/// If so, set the pipeline's final status.
async fn finalize_pipeline_if_done(
    pool: &sqlx::SqlitePool,
    pipeline_id: &str,
    streams: &delta_ci::PipelineStreams,
) {
    let jobs = match db::pipeline::list_jobs(pool, pipeline_id).await {
        Ok(j) => j,
        Err(_) => return,
    };

    let all_done = jobs.iter().all(|j| {
        matches!(
            j.status,
            db::pipeline::RunStatus::Passed
                | db::pipeline::RunStatus::Failed
                | db::pipeline::RunStatus::Cancelled
        )
    });

    if !all_done {
        return;
    }

    let all_passed = jobs
        .iter()
        .all(|j| j.status == db::pipeline::RunStatus::Passed);
    let final_status = if all_passed {
        db::pipeline::RunStatus::Passed
    } else {
        db::pipeline::RunStatus::Failed
    };

    let _ = db::pipeline::update_pipeline_status(pool, pipeline_id, final_status).await;

    if let Some(sender) = streams.get(pipeline_id) {
        let _ = sender.send(delta_ci::PipelineEvent::PipelineCompleted {
            status: format!("{:?}", final_status).to_lowercase(),
        });
    }
    streams.remove(pipeline_id);
}

