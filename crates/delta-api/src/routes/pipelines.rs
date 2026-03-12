//! Phase 4: CI/CD pipeline API routes.

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
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
    let run = db::pipeline::create_pipeline(
        &state.db,
        &repo.id.to_string(),
        &req.workflow_name,
        &req.trigger_type,
        req.trigger_ref.as_deref(),
        &req.commit_sha,
    )
    .await
    .map_err(|e| {
        tracing::error!("failed to create pipeline: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal server error".into(),
        )
    })?;
    Ok((StatusCode::CREATED, Json(run)))
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
    let encrypted = delta_core::crypto::encrypt(&encryption_key, req.value.as_bytes());
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
