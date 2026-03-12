use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::get,
};
use delta_core::db;
use delta_core::models::collaborator::CollaboratorRole;
use serde::{Deserialize, Serialize};

use crate::extractors::AuthUser;
use crate::helpers::{require_role, resolve_repo_authed};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/{owner}/{name}/collaborators",
            get(list_collaborators).post(add_collaborator),
        )
        .route(
            "/{owner}/{name}/collaborators/{username}",
            get(get_collaborator)
                .put(update_collaborator)
                .delete(remove_collaborator),
        )
}

#[derive(Serialize)]
struct CollaboratorResponse {
    username: String,
    role: String,
    created_at: String,
    updated_at: String,
}

async fn list_collaborators(
    State(state): State<AppState>,
    Path((owner, name)): Path<(String, String)>,
    AuthUser(user): AuthUser,
) -> Result<Json<Vec<CollaboratorResponse>>, (StatusCode, String)> {
    let (repo, owner_user) = resolve_repo_authed(&state, &owner, &name, &user).await?;
    require_role(&state, &repo, &owner_user, &user, CollaboratorRole::Admin).await?;

    let collabs = db::collaborator::list_for_repo(&state.db, &repo.id.to_string())
        .await
        .map_err(|e| {
            tracing::error!("failed to list collaborators: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal server error".into(),
            )
        })?;

    let mut responses = Vec::new();
    for c in collabs {
        let username = db::user::get_by_id(&state.db, &c.user_id.to_string())
            .await
            .map(|u| u.username)
            .unwrap_or_else(|_| c.user_id.to_string());
        responses.push(CollaboratorResponse {
            username,
            role: c.role.as_str().to_string(),
            created_at: c.created_at.to_rfc3339(),
            updated_at: c.updated_at.to_rfc3339(),
        });
    }

    Ok(Json(responses))
}

#[derive(Deserialize)]
struct AddCollaboratorRequest {
    username: String,
    #[serde(default = "default_role")]
    role: String,
}

fn default_role() -> String {
    "read".to_string()
}

async fn add_collaborator(
    State(state): State<AppState>,
    Path((owner, name)): Path<(String, String)>,
    AuthUser(user): AuthUser,
    Json(req): Json<AddCollaboratorRequest>,
) -> Result<(StatusCode, Json<CollaboratorResponse>), (StatusCode, String)> {
    let (repo, owner_user) = resolve_repo_authed(&state, &owner, &name, &user).await?;
    require_role(&state, &repo, &owner_user, &user, CollaboratorRole::Admin).await?;

    let role = CollaboratorRole::parse(&req.role)
        .ok_or((StatusCode::BAD_REQUEST, "invalid role: use read, write, or admin".into()))?;

    let target_user = db::user::get_by_username(&state.db, &req.username)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, format!("user '{}' not found", req.username)))?;

    // Cannot add the owner as a collaborator
    if target_user.id == owner_user.id {
        return Err((StatusCode::BAD_REQUEST, "owner cannot be added as collaborator".into()));
    }

    let collab = db::collaborator::set(
        &state.db,
        &repo.id.to_string(),
        &target_user.id.to_string(),
        role,
    )
    .await
    .map_err(|e| {
        tracing::error!("failed to add collaborator: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal server error".into(),
        )
    })?;

    let _ = db::audit::log(
        &state.db,
        Some(&user.id.to_string()),
        "add_collaborator",
        "repository",
        Some(&repo.id.to_string()),
        Some(&format!("{}/{} -> {} ({})", owner, name, req.username, req.role)),
        None,
    )
    .await;

    Ok((
        StatusCode::CREATED,
        Json(CollaboratorResponse {
            username: target_user.username,
            role: collab.role.as_str().to_string(),
            created_at: collab.created_at.to_rfc3339(),
            updated_at: collab.updated_at.to_rfc3339(),
        }),
    ))
}

async fn get_collaborator(
    State(state): State<AppState>,
    Path((owner, name, username)): Path<(String, String, String)>,
    AuthUser(user): AuthUser,
) -> Result<Json<CollaboratorResponse>, (StatusCode, String)> {
    let (repo, owner_user) = resolve_repo_authed(&state, &owner, &name, &user).await?;
    require_role(&state, &repo, &owner_user, &user, CollaboratorRole::Admin).await?;

    let target_user = db::user::get_by_username(&state.db, &username)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, format!("user '{}' not found", username)))?;

    let collab = db::collaborator::get(
        &state.db,
        &repo.id.to_string(),
        &target_user.id.to_string(),
    )
    .await
    .map_err(|_| (StatusCode::NOT_FOUND, "collaborator not found".into()))?;

    Ok(Json(CollaboratorResponse {
        username: target_user.username,
        role: collab.role.as_str().to_string(),
        created_at: collab.created_at.to_rfc3339(),
        updated_at: collab.updated_at.to_rfc3339(),
    }))
}

#[derive(Deserialize)]
struct UpdateCollaboratorRequest {
    role: String,
}

async fn update_collaborator(
    State(state): State<AppState>,
    Path((owner, name, username)): Path<(String, String, String)>,
    AuthUser(user): AuthUser,
    Json(req): Json<UpdateCollaboratorRequest>,
) -> Result<Json<CollaboratorResponse>, (StatusCode, String)> {
    let (repo, owner_user) = resolve_repo_authed(&state, &owner, &name, &user).await?;
    require_role(&state, &repo, &owner_user, &user, CollaboratorRole::Admin).await?;

    let role = CollaboratorRole::parse(&req.role)
        .ok_or((StatusCode::BAD_REQUEST, "invalid role: use read, write, or admin".into()))?;

    let target_user = db::user::get_by_username(&state.db, &username)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, format!("user '{}' not found", username)))?;

    let collab = db::collaborator::set(
        &state.db,
        &repo.id.to_string(),
        &target_user.id.to_string(),
        role,
    )
    .await
    .map_err(|e| {
        tracing::error!("failed to update collaborator: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal server error".into(),
        )
    })?;

    Ok(Json(CollaboratorResponse {
        username: target_user.username,
        role: collab.role.as_str().to_string(),
        created_at: collab.created_at.to_rfc3339(),
        updated_at: collab.updated_at.to_rfc3339(),
    }))
}

async fn remove_collaborator(
    State(state): State<AppState>,
    Path((owner, name, username)): Path<(String, String, String)>,
    AuthUser(user): AuthUser,
) -> Result<StatusCode, (StatusCode, String)> {
    let (repo, owner_user) = resolve_repo_authed(&state, &owner, &name, &user).await?;
    require_role(&state, &repo, &owner_user, &user, CollaboratorRole::Admin).await?;

    let target_user = db::user::get_by_username(&state.db, &username)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, format!("user '{}' not found", username)))?;

    db::collaborator::remove(
        &state.db,
        &repo.id.to_string(),
        &target_user.id.to_string(),
    )
    .await
    .map_err(|_| (StatusCode::NOT_FOUND, "collaborator not found".into()))?;

    let _ = db::audit::log(
        &state.db,
        Some(&user.id.to_string()),
        "remove_collaborator",
        "repository",
        Some(&repo.id.to_string()),
        Some(&format!("{}/{} -> {}", owner, name, username)),
        None,
    )
    .await;

    Ok(StatusCode::NO_CONTENT)
}
