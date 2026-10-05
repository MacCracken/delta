//! Phase 4: CI/CD pipeline API routes.

use axum::{
    Json, Router,
    extract::{Path, Query, State, WebSocketUpgrade, ws},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
};
use delta_core::db;
use serde::{Deserialize, Serialize};

use delta_core::models::collaborator::CollaboratorRole;

use crate::extractors::AuthUser;
use crate::helpers::{require_role, resolve_repo_authed};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/{owner}/{name}/pipelines",
            get(list_pipelines).post(trigger_pipeline),
        )
        .route("/{owner}/{name}/pipelines/{pipeline_id}", get(get_pipeline))
        .route(
            "/{owner}/{name}/pipelines/{pipeline_id}/cancel",
            axum::routing::post(cancel_pipeline),
        )
        .route(
            "/{owner}/{name}/pipelines/{pipeline_id}/jobs",
            get(list_jobs),
        )
        .route(
            "/{owner}/{name}/pipelines/{pipeline_id}/jobs/{job_id}/logs",
            get(get_job_logs),
        )
        .route(
            "/{owner}/{name}/pipelines/{pipeline_id}/ws",
            get(stream_pipeline_logs),
        )
        .route(
            "/{owner}/{name}/secrets",
            get(list_secrets).post(set_secret),
        )
        .route(
            "/{owner}/{name}/secrets/{secret_name}",
            axum::routing::delete(delete_secret),
        )
}

#[derive(Deserialize)]
struct ListPipelinesQuery {
    status: Option<String>,
    #[serde(default = "default_limit")]
    limit: i64,
}
fn default_limit() -> i64 {
    50
}

async fn list_pipelines(
    State(state): State<AppState>,
    Path((owner, name)): Path<(String, String)>,
    AuthUser(user): AuthUser,
    Query(query): Query<ListPipelinesQuery>,
) -> Result<Json<Vec<db::pipeline::PipelineRun>>, (StatusCode, String)> {
    let (repo, _) = resolve_repo_authed(&state, &owner, &name, &user).await?;
    let limit = query.limit.clamp(1, 200);
    let runs = db::pipeline::list_pipelines(
        &state.db,
        &repo.id.to_string(),
        query.status.as_deref(),
        limit,
    )
    .await
    .map_err(|e| {
        tracing::error!("failed to list pipelines: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal server error".into(),
        )
    })?;
    Ok(Json(runs))
}

#[derive(Deserialize)]
struct TriggerPipelineRequest {
    workflow_name: String,
    commit_sha: String,
    #[serde(default = "default_trigger")]
    trigger_type: String,
    trigger_ref: Option<String>,
}
fn default_trigger() -> String {
    "manual".into()
}

async fn trigger_pipeline(
    State(state): State<AppState>,
    Path((owner, name)): Path<(String, String)>,
    AuthUser(user): AuthUser,
    Json(req): Json<TriggerPipelineRequest>,
) -> Result<(StatusCode, Json<db::pipeline::PipelineRun>), (StatusCode, String)> {
    let (repo, owner_user) = resolve_repo_authed(&state, &owner, &name, &user).await?;
    require_role(&state, &repo, &owner_user, &user, CollaboratorRole::Write).await?;
    if req.trigger_type.is_empty()
        || req.trigger_type.len() > 32
        || !req
            .trigger_type
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err((StatusCode::BAD_REQUEST, "invalid trigger_type".into()));
    }
    let repo_path = state
        .repo_host
        .repo_path(&owner, &name)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    // Runs exactly the commit asked for (a commit id, branch or tag).
    let commit_sha = delta_vcs::refs::resolve_commit(&repo_path, &req.commit_sha)
        .await
        .ok_or((StatusCode::BAD_REQUEST, "commit not found".to_string()))?;
    let run = start_manual_pipeline(
        &state,
        &repo,
        repo_path,
        &req.workflow_name,
        &req.trigger_type,
        req.trigger_ref.as_deref(),
        &commit_sha,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(run)))
}

/// Create a queued run of `workflow_name` at `commit_sha` and start it in
/// the background, in a checkout of that commit.
pub(crate) async fn start_manual_pipeline(
    state: &AppState,
    repo: &delta_core::models::repo::Repository,
    repo_path: std::path::PathBuf,
    workflow_name: &str,
    trigger_type: &str,
    trigger_ref: Option<&str>,
    commit_sha: &str,
) -> Result<db::pipeline::PipelineRun, (StatusCode, String)> {
    if workflow_name.is_empty() || workflow_name.len() > 255 {
        return Err((StatusCode::BAD_REQUEST, "invalid workflow_name".into()));
    }
    let run = db::pipeline::create_pipeline(
        &state.db,
        &repo.id.to_string(),
        workflow_name,
        trigger_type,
        trigger_ref,
        commit_sha,
    )
    .await
    .map_err(|e| {
        tracing::error!("failed to create pipeline: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal server error".into(),
        )
    })?;

    let state = state.clone();
    let repo = repo.clone();
    let pipeline_id = run.id.clone();
    let workflow_name = workflow_name.to_string();
    let trigger_type = trigger_type.to_string();
    let trigger_ref = trigger_ref.map(str::to_string);
    let commit_sha = commit_sha.to_string();
    tokio::spawn(async move {
        let target = crate::routes::git::PipelineTarget::Run {
            pipeline_id: &pipeline_id,
            workflow_name: &workflow_name,
            trigger_type: &trigger_type,
            trigger_ref: trigger_ref.as_deref(),
        };
        if let Err(e) = crate::routes::git::run_pipelines_at_commit(
            &state,
            &repo,
            &repo_path,
            &commit_sha,
            target,
        )
        .await
        {
            tracing::warn!(pipeline_id, "pipeline could not start: {}", e);
            let _ = db::pipeline::update_pipeline_status(
                &state.db,
                &pipeline_id,
                db::pipeline::RunStatus::Failed,
            )
            .await;
        }
    });
    Ok(run)
}

async fn get_pipeline(
    State(state): State<AppState>,
    Path((owner, name, pipeline_id)): Path<(String, String, String)>,
    AuthUser(user): AuthUser,
) -> Result<Json<db::pipeline::PipelineRun>, (StatusCode, String)> {
    let (repo, _) = resolve_repo_authed(&state, &owner, &name, &user).await?;
    let run = db::pipeline::get_pipeline(&state.db, &pipeline_id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    if run.repo_id != repo.id.to_string() {
        return Err((StatusCode::NOT_FOUND, "pipeline not found".into()));
    }
    Ok(Json(run))
}

async fn cancel_pipeline(
    State(state): State<AppState>,
    Path((owner, name, pipeline_id)): Path<(String, String, String)>,
    AuthUser(user): AuthUser,
) -> Result<Json<db::pipeline::PipelineRun>, (StatusCode, String)> {
    let (repo, owner_user) = resolve_repo_authed(&state, &owner, &name, &user).await?;
    require_role(&state, &repo, &owner_user, &user, CollaboratorRole::Write).await?;
    let existing = db::pipeline::get_pipeline(&state.db, &pipeline_id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    if existing.repo_id != repo.id.to_string() {
        return Err((StatusCode::NOT_FOUND, "pipeline not found".into()));
    }
    let run = db::pipeline::update_pipeline_status(
        &state.db,
        &pipeline_id,
        db::pipeline::RunStatus::Cancelled,
    )
    .await
    .map_err(|e| {
        tracing::error!("failed to cancel pipeline: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal server error".into(),
        )
    })?;
    // Stop handing its jobs to self-hosted runners.
    if let Err(e) = db::runner::cancel_pipeline_jobs(&state.db, &pipeline_id).await {
        tracing::error!("failed to cancel queued jobs: {}", e);
    }
    Ok(Json(run))
}

async fn list_jobs(
    State(state): State<AppState>,
    Path((owner, name, pipeline_id)): Path<(String, String, String)>,
    AuthUser(user): AuthUser,
) -> Result<Json<Vec<db::pipeline::JobRun>>, (StatusCode, String)> {
    let (repo, _) = resolve_repo_authed(&state, &owner, &name, &user).await?;
    let run = db::pipeline::get_pipeline(&state.db, &pipeline_id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    if run.repo_id != repo.id.to_string() {
        return Err((StatusCode::NOT_FOUND, "pipeline not found".into()));
    }
    let jobs = db::pipeline::list_jobs(&state.db, &pipeline_id)
        .await
        .map_err(|e| {
            tracing::error!("failed to list jobs: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal server error".into(),
            )
        })?;
    Ok(Json(jobs))
}

async fn get_job_logs(
    State(state): State<AppState>,
    Path((owner, name, pipeline_id, job_id)): Path<(String, String, String, String)>,
    AuthUser(user): AuthUser,
) -> Result<Json<Vec<db::pipeline::StepLog>>, (StatusCode, String)> {
    let (repo, _) = resolve_repo_authed(&state, &owner, &name, &user).await?;
    let run = db::pipeline::get_pipeline(&state.db, &pipeline_id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    if run.repo_id != repo.id.to_string() {
        return Err((StatusCode::NOT_FOUND, "pipeline not found".into()));
    }
    // The job must belong to the pipeline checked above, or any job's logs
    // could be read through any accessible pipeline.
    let job = db::pipeline::get_job(&state.db, &job_id)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "job not found".to_string()))?;
    if job.pipeline_id != run.id {
        return Err((StatusCode::NOT_FOUND, "job not found".into()));
    }
    let logs = db::pipeline::get_step_logs(&state.db, &job_id)
        .await
        .map_err(|e| {
            tracing::error!("failed to get step logs: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal server error".into(),
            )
        })?;
    Ok(Json(logs))
}

// --- WebSocket log streaming ---

#[derive(Deserialize)]
struct WsQuery {
    token: Option<String>,
}

async fn stream_pipeline_logs(
    State(state): State<AppState>,
    Path((owner, name, pipeline_id)): Path<(String, String, String)>,
    Query(query): Query<WsQuery>,
    ws: WebSocketUpgrade,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    // Browsers can't set headers on a WebSocket, so a token comes as a query
    // parameter. Without one, only public repositories' pipelines stream:
    // their logs are public on the web UI too.
    let (repo, _) = match query.token.as_deref() {
        Some(token) => {
            let user = crate::auth::authenticate_token(&state.db, token)
                .await
                .map_err(|_| (StatusCode::UNAUTHORIZED, "invalid or expired token".into()))?;
            resolve_repo_authed(&state, &owner, &name, &user).await?
        }
        None => crate::helpers::resolve_repo(&state, &owner, &name).await?,
    };
    let run = db::pipeline::get_pipeline(&state.db, &pipeline_id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    if run.repo_id != repo.id.to_string() {
        return Err((StatusCode::NOT_FOUND, "pipeline not found".into()));
    }

    let streams = state.pipeline_streams.clone();
    let db = state.db.clone();
    let pid = pipeline_id.clone();

    Ok(ws.on_upgrade(move |socket| handle_pipeline_ws(socket, streams, db, pid)))
}

async fn handle_pipeline_ws(
    mut socket: ws::WebSocket,
    streams: delta_ci::PipelineStreams,
    db: sqlx::SqlitePool,
    pipeline_id: String,
) {
    // Try to subscribe to the live broadcast channel
    if let Some(sender) = streams.get(&pipeline_id) {
        let mut rx = sender.subscribe();
        drop(sender); // Release DashMap ref

        loop {
            tokio::select! {
                event = rx.recv() => {
                    match event {
                        Ok(evt) => {
                            let json = match serde_json::to_string(&evt) {
                                Ok(j) => j,
                                Err(_) => continue,
                            };
                            if socket.send(ws::Message::Text(json.into())).await.is_err() {
                                return; // Client disconnected
                            }
                            // If pipeline completed, we're done
                            if matches!(evt, delta_ci::PipelineEvent::PipelineCompleted { .. }) {
                                let _ = socket.send(ws::Message::Close(None)).await;
                                return;
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                            tracing::warn!(pipeline_id = %pipeline_id, "ws client lagged by {} events", n);
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                            break; // Channel closed, pipeline finished
                        }
                    }
                }
                msg = socket.recv() => {
                    match msg {
                        Some(Ok(ws::Message::Close(_))) | None => return,
                        _ => {} // Ignore other messages
                    }
                }
            }
        }
    }

    // Pipeline already finished (or just finished) — send historical logs from DB
    let jobs = db::pipeline::list_jobs(&db, &pipeline_id)
        .await
        .unwrap_or_default();
    for job in &jobs {
        let logs = db::pipeline::get_step_logs(&db, &job.id)
            .await
            .unwrap_or_default();
        for log in &logs {
            let evt = serde_json::json!({
                "type": "step_output",
                "job_id": job.id,
                "step_index": log.step_index,
                "line": log.output,
            });
            if let Ok(json) = serde_json::to_string(&evt)
                && socket.send(ws::Message::Text(json.into())).await.is_err()
            {
                return;
            }
        }
    }

    // Send pipeline completed
    let run = db::pipeline::get_pipeline(&db, &pipeline_id).await;
    let status = run
        .map(|r| format!("{:?}", r.status).to_lowercase())
        .unwrap_or_else(|_| "unknown".into());
    let evt = serde_json::json!({
        "type": "pipeline_completed",
        "status": status,
    });
    if let Ok(json) = serde_json::to_string(&evt) {
        let _ = socket.send(ws::Message::Text(json.into())).await;
    }
    let _ = socket.send(ws::Message::Close(None)).await;
}

// --- Secrets ---

async fn list_secrets(
    State(state): State<AppState>,
    Path((owner, name)): Path<(String, String)>,
    AuthUser(user): AuthUser,
) -> Result<Json<Vec<SecretResponse>>, (StatusCode, String)> {
    let (repo, owner_user) = resolve_repo_authed(&state, &owner, &name, &user).await?;
    require_role(&state, &repo, &owner_user, &user, CollaboratorRole::Admin).await?;
    let secrets = db::secret::list(&state.db, &repo.id.to_string())
        .await
        .map_err(|e| {
            tracing::error!("failed to list secrets: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal server error".into(),
            )
        })?;
    Ok(Json(
        secrets
            .into_iter()
            .map(|s| SecretResponse {
                name: s.name,
                created_at: s.created_at,
                updated_at: s.updated_at,
            })
            .collect(),
    ))
}

#[derive(Serialize)]
struct SecretResponse {
    name: String,
    created_at: String,
    updated_at: String,
}

#[derive(Deserialize)]
struct SetSecretRequest {
    name: String,
    value: String,
}

async fn set_secret(
    State(state): State<AppState>,
    Path((owner, name)): Path<(String, String)>,
    AuthUser(user): AuthUser,
    Json(req): Json<SetSecretRequest>,
) -> Result<(StatusCode, Json<SecretResponse>), (StatusCode, String)> {
    // Validate secret name: 1-256 chars, alphanumeric/underscores/hyphens
    if req.name.is_empty()
        || req.name.len() > 256
        || !req
            .name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "secret name must be 1-256 alphanumeric characters, underscores, or hyphens".into(),
        ));
    }
    if req.value.is_empty() || req.value.len() > 65536 {
        return Err((
            StatusCode::BAD_REQUEST,
            "secret value must be 1-65536 characters".into(),
        ));
    }

    let (repo, owner_user) = resolve_repo_authed(&state, &owner, &name, &user).await?;
    require_role(&state, &repo, &owner_user, &user, CollaboratorRole::Admin).await?;
    let encryption_key = delta_core::crypto::derive_key(&state.config.auth.secrets_key);
    let encrypted =
        delta_core::crypto::encrypt(&encryption_key, req.value.as_bytes()).map_err(|e| {
            tracing::error!("encryption failed: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal server error".into(),
            )
        })?;
    let repo_id = repo.id.to_string();
    db::secret::set(&state.db, &repo_id, &req.name, &encrypted)
        .await
        .map_err(|e| {
            tracing::error!("failed to set secret: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal server error".into(),
            )
        })?;

    // Retrieve the saved secret metadata for the response
    let secrets = db::secret::list(&state.db, &repo_id).await.map_err(|e| {
        tracing::error!("failed to list secrets: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal server error".into(),
        )
    })?;
    let saved = secrets.into_iter().find(|s| s.name == req.name).ok_or((
        StatusCode::INTERNAL_SERVER_ERROR,
        "secret saved but not found".into(),
    ))?;

    Ok((
        StatusCode::CREATED,
        Json(SecretResponse {
            name: saved.name,
            created_at: saved.created_at,
            updated_at: saved.updated_at,
        }),
    ))
}

async fn delete_secret(
    State(state): State<AppState>,
    Path((owner, name, secret_name)): Path<(String, String, String)>,
    AuthUser(user): AuthUser,
) -> Result<StatusCode, (StatusCode, String)> {
    let (repo, owner_user) = resolve_repo_authed(&state, &owner, &name, &user).await?;
    require_role(&state, &repo, &owner_user, &user, CollaboratorRole::Admin).await?;
    db::secret::delete(&state.db, &repo.id.to_string(), &secret_name)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}
