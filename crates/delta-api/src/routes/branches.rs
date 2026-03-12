use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::get,
};
use serde::{Deserialize, Serialize};

use delta_core::models::collaborator::CollaboratorRole;

use crate::extractors::AuthUser;
use crate::helpers::{require_role, resolve_repo_authed};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/{owner}/{name}/branches", get(list_branches))
        .route("/{owner}/{name}/tags", get(list_tags))
        .route(
            "/{owner}/{name}/branch-protections",
            get(list_protections).post(create_protection),
        )
        .route(
            "/{owner}/{name}/branch-protections/{protection_id}",
            axum::routing::delete(delete_protection),
        )
}

async fn list_branches(
    State(state): State<AppState>,
    Path((owner, name)): Path<(String, String)>,
    AuthUser(user): AuthUser,
) -> Result<Json<Vec<delta_vcs::refs::BranchInfo>>, (StatusCode, String)> {
    // Enforce visibility check for private repos
    let _ = resolve_repo_authed(&state, &owner, &name, &user).await?;

    let repo_path = state
        .repo_host
        .repo_path(&owner, &name)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    if !repo_path.exists() {
        return Err((StatusCode::NOT_FOUND, "repository not found".into()));
    }

    let branches = delta_vcs::refs::list_branches(&repo_path)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(branches))
}

async fn list_tags(
    State(state): State<AppState>,
    Path((owner, name)): Path<(String, String)>,
    AuthUser(user): AuthUser,
) -> Result<Json<Vec<delta_vcs::refs::TagInfo>>, (StatusCode, String)> {
    // Enforce visibility check for private repos
    let _ = resolve_repo_authed(&state, &owner, &name, &user).await?;

    let repo_path = state
        .repo_host
        .repo_path(&owner, &name)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    if !repo_path.exists() {
        return Err((StatusCode::NOT_FOUND, "repository not found".into()));
    }

    let tags = delta_vcs::refs::list_tags(&repo_path)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(tags))
}

#[derive(Serialize)]
struct ProtectionResponse {
    id: String,
    pattern: String,
    require_pr: bool,
    required_approvals: u32,
    require_status_checks: bool,
    prevent_force_push: bool,
    prevent_deletion: bool,
}

async fn list_protections(
    State(state): State<AppState>,
    Path((owner, name)): Path<(String, String)>,
    AuthUser(user): AuthUser,
) -> Result<Json<Vec<ProtectionResponse>>, (StatusCode, String)> {
    let owner_user = delta_core::db::user::get_by_username(&state.db, &owner)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "user not found".into()))?;

    let owner_id = owner_user.id.to_string();
    let repo = delta_core::db::repo::get_by_owner_and_name(&state.db, &owner_id, &name)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "repository not found".into()))?;

    require_role(&state, &repo, &owner_user, &user, CollaboratorRole::Admin).await?;

    let protections =
        delta_core::db::branch_protection::list_for_repo(&state.db, &repo.id.to_string())
            .await
            .map_err(|e| {
                tracing::error!("failed to list branch protections: {}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal server error".into(),
                )
            })?;

    Ok(Json(
        protections
            .into_iter()
            .map(|p| ProtectionResponse {
                id: p.id.to_string(),
                pattern: p.pattern,
                require_pr: p.require_pr,
                required_approvals: p.required_approvals,
                require_status_checks: p.require_status_checks,
                prevent_force_push: p.prevent_force_push,
                prevent_deletion: p.prevent_deletion,
            })
            .collect(),
    ))
}

#[derive(Deserialize)]
struct CreateProtectionRequest {
    pattern: String,
    #[serde(default)]
    require_pr: bool,
    #[serde(default)]
    required_approvals: u32,
    #[serde(default)]
    require_status_checks: bool,
    #[serde(default = "default_true")]
    prevent_force_push: bool,
    #[serde(default = "default_true")]
    prevent_deletion: bool,
}

fn default_true() -> bool {
    true
}

async fn create_protection(
    State(state): State<AppState>,
    Path((owner, name)): Path<(String, String)>,
    AuthUser(user): AuthUser,
    Json(req): Json<CreateProtectionRequest>,
) -> Result<(StatusCode, Json<ProtectionResponse>), (StatusCode, String)> {
    let owner_user = delta_core::db::user::get_by_username(&state.db, &owner)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "user not found".into()))?;

    let owner_id = owner_user.id.to_string();
    let repo = delta_core::db::repo::get_by_owner_and_name(&state.db, &owner_id, &name)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "repository not found".into()))?;

    require_role(&state, &repo, &owner_user, &user, CollaboratorRole::Admin).await?;

    let protection = delta_core::db::branch_protection::create(
        &state.db,
        delta_core::db::branch_protection::CreateParams {
            repo_id: &repo.id.to_string(),
            pattern: &req.pattern,
            require_pr: req.require_pr,
            required_approvals: req.required_approvals,
            require_status_checks: req.require_status_checks,
            prevent_force_push: req.prevent_force_push,
            prevent_deletion: req.prevent_deletion,
        },
    )
    .await
    .map_err(|e| (StatusCode::CONFLICT, e.to_string()))?;

    Ok((
        StatusCode::CREATED,
        Json(ProtectionResponse {
            id: protection.id.to_string(),
            pattern: protection.pattern,
            require_pr: protection.require_pr,
            required_approvals: protection.required_approvals,
            require_status_checks: protection.require_status_checks,
            prevent_force_push: protection.prevent_force_push,
            prevent_deletion: protection.prevent_deletion,
        }),
    ))
}

async fn delete_protection(
    State(state): State<AppState>,
    Path((owner, name, protection_id)): Path<(String, String, String)>,
    AuthUser(user): AuthUser,
) -> Result<StatusCode, (StatusCode, String)> {
    let owner_user = delta_core::db::user::get_by_username(&state.db, &owner)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "user not found".into()))?;

    let owner_id = owner_user.id.to_string();
    let repo = delta_core::db::repo::get_by_owner_and_name(&state.db, &owner_id, &name)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "repository not found".into()))?;

    require_role(&state, &repo, &owner_user, &user, CollaboratorRole::Admin).await?;

    // Verify protection belongs to this repo before deleting
    let protections =
        delta_core::db::branch_protection::list_for_repo(&state.db, &repo.id.to_string())
            .await
            .map_err(|e| {
                tracing::error!("failed to list branch protections: {}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal server error".into(),
                )
            })?;
    if !protections
        .iter()
        .any(|p| p.id.to_string() == protection_id)
    {
        return Err((StatusCode::NOT_FOUND, "protection rule not found".into()));
    }

    delta_core::db::branch_protection::delete(&state.db, &protection_id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;

    Ok(StatusCode::NO_CONTENT)
}
