use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::get,
};
use delta_core::db;
use delta_core::models::repo::Visibility;
use serde::{Deserialize, Serialize};

use crate::extractors::{AuthUser, MaybeAuthUser};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_repos).post(create_repo))
        .route("/{owner}/{name}", get(get_repo).put(update_repo).delete(delete_repo))
}

#[derive(Serialize)]
struct RepoResponse {
    id: String,
    owner: String,
    name: String,
    full_name: String,
    description: Option<String>,
    visibility: String,
    default_branch: String,
    created_at: String,
    updated_at: String,
}

impl RepoResponse {
    fn from_repo(repo: delta_core::models::repo::Repository, owner_name: &str) -> Self {
        Self {
            id: repo.id.to_string(),
            full_name: format!("{}/{}", owner_name, repo.name),
            owner: owner_name.to_string(),
            name: repo.name,
            description: repo.description,
            visibility: repo.visibility.as_str().to_string(),
            default_branch: repo.default_branch,
            created_at: repo.created_at.to_rfc3339(),
            updated_at: repo.updated_at.to_rfc3339(),
        }
    }
}

async fn list_repos(
    State(state): State<AppState>,
    MaybeAuthUser(user): MaybeAuthUser,
) -> Result<Json<Vec<RepoResponse>>, (StatusCode, String)> {
    let viewer_id = user.as_ref().map(|u| u.id.to_string());
    let repos = db::repo::list_visible(&state.db, viewer_id.as_deref())
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // We need owner usernames — for now use the owner field (which stores owner_id)
    // TODO: join with users table for proper username resolution
    let mut responses = Vec::new();
    for repo in repos {
        let owner_name = db::user::get_by_id(&state.db, &repo.owner)
            .await
            .map(|u| u.username)
            .unwrap_or_else(|_| repo.owner.clone());
        responses.push(RepoResponse::from_repo(repo, &owner_name));
    }

    Ok(Json(responses))
}

#[derive(Deserialize)]
struct CreateRepoRequest {
    name: String,
    description: Option<String>,
    #[serde(default = "default_visibility")]
    visibility: String,
}

fn default_visibility() -> String {
    "private".to_string()
}

async fn create_repo(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Json(req): Json<CreateRepoRequest>,
) -> Result<(StatusCode, Json<RepoResponse>), (StatusCode, String)> {
    if req.name.is_empty()
        || req.name.len() > 100
        || req.name.starts_with('-')
        || req.name.starts_with('.')
        || !req.name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        return Err((StatusCode::BAD_REQUEST, "invalid repository name".into()));
    }
    let visibility = match req.visibility.as_str() {
        "public" => Visibility::Public,
        "internal" => Visibility::Internal,
        _ => Visibility::Private,
    };

    let user_id = user.id.to_string();
    let repo = db::repo::create(
        &state.db,
        &user_id,
        &req.name,
        req.description.as_deref(),
        visibility,
    )
    .await
    .map_err(|e| (StatusCode::CONFLICT, e.to_string()))?;

    // Initialize bare git repo on disk
    if let Err(e) = state.repo_host.init_bare(&user.username, &req.name) {
        tracing::warn!("failed to init bare repo on disk: {}", e);
    }

    Ok((
        StatusCode::CREATED,
        Json(RepoResponse::from_repo(repo, &user.username)),
    ))
}

async fn get_repo(
    State(state): State<AppState>,
    MaybeAuthUser(user): MaybeAuthUser,
    Path((owner, name)): Path<(String, String)>,
) -> Result<Json<RepoResponse>, (StatusCode, String)> {
    // Look up owner by username
    let owner_user = db::user::get_by_username(&state.db, &owner)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, format!("user '{}' not found", owner)))?;

    let owner_id = owner_user.id.to_string();
    let repo = db::repo::get_by_owner_and_name(&state.db, &owner_id, &name)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, format!("repository '{}/{}' not found", owner, name)))?;

    // Check visibility
    if repo.visibility == Visibility::Private {
        let is_owner = user.as_ref().is_some_and(|u| u.id == owner_user.id);
        if !is_owner {
            return Err((StatusCode::NOT_FOUND, format!("repository '{}/{}' not found", owner, name)));
        }
    }

    Ok(Json(RepoResponse::from_repo(repo, &owner)))
}

#[derive(Deserialize)]
struct UpdateRepoRequest {
    description: Option<String>,
    visibility: Option<String>,
    default_branch: Option<String>,
}

async fn update_repo(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path((owner, name)): Path<(String, String)>,
    Json(req): Json<UpdateRepoRequest>,
) -> Result<Json<RepoResponse>, (StatusCode, String)> {
    let owner_user = db::user::get_by_username(&state.db, &owner)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, format!("user '{}' not found", owner)))?;

    if user.id != owner_user.id {
        return Err((StatusCode::FORBIDDEN, "not the repository owner".into()));
    }

    let owner_id = owner_user.id.to_string();
    let repo = db::repo::get_by_owner_and_name(&state.db, &owner_id, &name)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "repository not found".to_string()))?;

    let visibility = req.visibility.as_deref().map(|v| match v {
        "public" => Visibility::Public,
        "internal" => Visibility::Internal,
        _ => Visibility::Private,
    });

    let updated = db::repo::update(
        &state.db,
        &repo.id.to_string(),
        req.description.as_deref(),
        visibility,
        req.default_branch.as_deref(),
    )
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(RepoResponse::from_repo(updated, &owner)))
}

async fn delete_repo(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path((owner, name)): Path<(String, String)>,
) -> Result<StatusCode, (StatusCode, String)> {
    let owner_user = db::user::get_by_username(&state.db, &owner)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, format!("user '{}' not found", owner)))?;

    if user.id != owner_user.id {
        return Err((StatusCode::FORBIDDEN, "not the repository owner".into()));
    }

    let owner_id = owner_user.id.to_string();
    let repo = db::repo::get_by_owner_and_name(&state.db, &owner_id, &name)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "repository not found".into()))?;

    db::repo::delete(&state.db, &repo.id.to_string())
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Remove from disk
    if let Err(e) = state.repo_host.delete(&owner, &name) {
        tracing::warn!("failed to delete repo from disk: {}", e);
    }

    Ok(StatusCode::NO_CONTENT)
}
