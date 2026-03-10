use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::get,
};
use delta_core::db;
use delta_core::models::pull_request::CheckState;
use delta_core::models::repo::Visibility;
use serde::{Deserialize, Serialize};

use crate::extractors::AuthUser;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new().route(
        "/{owner}/{name}/commits/{sha}/statuses",
        get(list_statuses).post(create_status),
    )
}

#[derive(Serialize)]
struct StatusResponse {
    id: String,
    context: String,
    state: String,
    description: Option<String>,
    target_url: Option<String>,
    created_at: String,
    updated_at: String,
}

async fn list_statuses(
    State(state): State<AppState>,
    Path((owner, name, sha)): Path<(String, String, String)>,
) -> Result<Json<Vec<StatusResponse>>, (StatusCode, String)> {
    let owner_user = db::user::get_by_username(&state.db, &owner)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "user not found".into()))?;
    let owner_id = owner_user.id.to_string();
    let repo = db::repo::get_by_owner_and_name(&state.db, &owner_id, &name)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "repository not found".into()))?;

    // Reject requests for private repos on unauthenticated endpoint
    if repo.visibility != Visibility::Public {
        return Err((StatusCode::NOT_FOUND, "repository not found".into()));
    }

    let checks = db::status_check::get_for_commit(&state.db, &repo.id.to_string(), &sha)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(
        checks
            .into_iter()
            .map(|c| StatusResponse {
                id: c.id.to_string(),
                context: c.context,
                state: c.state.as_str().to_string(),
                description: c.description,
                target_url: c.target_url,
                created_at: c.created_at.to_rfc3339(),
                updated_at: c.updated_at.to_rfc3339(),
            })
            .collect(),
    ))
}

#[derive(Deserialize)]
struct CreateStatusRequest {
    context: String,
    state: String,
    description: Option<String>,
    target_url: Option<String>,
}

async fn create_status(
    State(state): State<AppState>,
    Path((owner, name, sha)): Path<(String, String, String)>,
    AuthUser(_user): AuthUser,
    Json(req): Json<CreateStatusRequest>,
) -> Result<(StatusCode, Json<StatusResponse>), (StatusCode, String)> {
    let owner_user = db::user::get_by_username(&state.db, &owner)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "user not found".into()))?;
    let owner_id = owner_user.id.to_string();
    let repo = db::repo::get_by_owner_and_name(&state.db, &owner_id, &name)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "repository not found".into()))?;

    let check_state = match req.state.as_str() {
        "success" => CheckState::Success,
        "failure" => CheckState::Failure,
        "error" => CheckState::Error,
        _ => CheckState::Pending,
    };

    let check = db::status_check::upsert(
        &state.db,
        &repo.id.to_string(),
        &sha,
        &req.context,
        check_state,
        req.description.as_deref(),
        req.target_url.as_deref(),
    )
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok((
        StatusCode::CREATED,
        Json(StatusResponse {
            id: check.id.to_string(),
            context: check.context,
            state: check.state.as_str().to_string(),
            description: check.description,
            target_url: check.target_url,
            created_at: check.created_at.to_rfc3339(),
            updated_at: check.updated_at.to_rfc3339(),
        }),
    ))
}
