use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::post,
};
use delta_core::db;
use delta_core::models::repo::Visibility;
use serde::{Deserialize, Serialize};

use crate::extractors::{AuthUser, MaybeAuthUser};
use crate::helpers::resolve_repo_authed;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new().route("/{owner}/{name}/forks", post(create_fork).get(list_forks))
}

#[derive(Deserialize)]
struct ForkRequest {
    /// Optional: rename the fork (defaults to source repo name).
    name: Option<String>,
    #[serde(default = "default_visibility")]
    visibility: String,
}

fn default_visibility() -> String {
    "private".to_string()
}

#[derive(Serialize)]
struct ForkResponse {
    id: String,
    owner: String,
    name: String,
    full_name: String,
    description: Option<String>,
    visibility: String,
    default_branch: String,
    forked_from: Option<String>,
    created_at: String,
    updated_at: String,
}

impl ForkResponse {
    fn from_repo(repo: delta_core::models::repo::Repository, owner_name: &str) -> Self {
        Self {
            id: repo.id.to_string(),
            full_name: format!("{}/{}", owner_name, repo.name),
            owner: owner_name.to_string(),
            name: repo.name,
            description: repo.description,
            visibility: repo.visibility.as_str().to_string(),
            default_branch: repo.default_branch,
            forked_from: repo.forked_from.map(|id| id.to_string()),
            created_at: repo.created_at.to_rfc3339(),
            updated_at: repo.updated_at.to_rfc3339(),
        }
    }
}

async fn create_fork(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path((owner, name)): Path<(String, String)>,
    Json(req): Json<ForkRequest>,
) -> Result<(StatusCode, Json<ForkResponse>), (StatusCode, String)> {
    // Resolve source repo (respects visibility)
    let (source_repo, _owner_user) = resolve_repo_authed(&state, &owner, &name, &user).await?;

    let fork_name = req.name.as_deref().unwrap_or(&name);

    // Validate fork name
    if fork_name.is_empty()
        || fork_name.len() > 100
        || fork_name.starts_with('-')
        || fork_name.starts_with('.')
        || !fork_name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        return Err((StatusCode::BAD_REQUEST, "invalid repository name".into()));
    }

    // Cannot fork your own repo
    let user_id = user.id.to_string();
    if source_repo.owner == user_id {
        return Err((
            StatusCode::BAD_REQUEST,
            "cannot fork your own repository".into(),
        ));
    }

    let visibility = match req.visibility.as_str() {
        "public" => Visibility::Public,
        "internal" => Visibility::Internal,
        _ => Visibility::Private,
    };

    // Create fork record in DB
    let fork = db::repo::create_fork(
        &state.db,
        &user_id,
        fork_name,
        source_repo.description.as_deref(),
        visibility,
        &source_repo.id.to_string(),
    )
    .await
    .map_err(|e| (StatusCode::CONFLICT, e.to_string()))?;

    // Clone bare repo on disk
    if let Err(e) = state
        .repo_host
        .clone_bare(&owner, &name, &user.username, fork_name)
    {
        // Rollback DB record on disk failure
        let _ = db::repo::delete(&state.db, &fork.id.to_string()).await;
        tracing::error!("failed to clone bare repo for fork: {}", e);
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            "failed to create fork on disk".into(),
        ));
    }

    // Audit
    let _ = delta_core::db::audit::log(
        &state.db,
        Some(&user_id),
        "fork",
        "repository",
        Some(&fork.id.to_string()),
        Some(&format!(
            "{}/{} -> {}/{}",
            owner, name, user.username, fork_name
        )),
        None,
    )
    .await;

    Ok((
        StatusCode::CREATED,
        Json(ForkResponse::from_repo(fork, &user.username)),
    ))
}

async fn list_forks(
    State(state): State<AppState>,
    MaybeAuthUser(user): MaybeAuthUser,
    Path((owner, name)): Path<(String, String)>,
) -> Result<Json<Vec<ForkResponse>>, (StatusCode, String)> {
    // Look up the source repo
    let owner_user = db::user::get_by_username(&state.db, &owner)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "user not found".into()))?;

    let owner_id = owner_user.id.to_string();
    let repo = db::repo::get_by_owner_and_name(&state.db, &owner_id, &name)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "repository not found".into()))?;

    // Public repos: anyone can see forks. Private: require auth + access.
    if repo.visibility != Visibility::Public {
        let u = user
            .as_ref()
            .ok_or((StatusCode::NOT_FOUND, "repository not found".into()))?;
        let is_owner = u.id == owner_user.id;
        if !is_owner {
            let is_collab =
                db::collaborator::get_role(&state.db, &repo.id.to_string(), &u.id.to_string())
                    .await
                    .unwrap_or(None)
                    .is_some();
            if !is_collab {
                return Err((StatusCode::NOT_FOUND, "repository not found".into()));
            }
        }
    }

    let forks = db::repo::list_forks(&state.db, &repo.id.to_string())
        .await
        .map_err(|e| {
            tracing::error!("failed to list forks: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal server error".into(),
            )
        })?;

    let mut responses = Vec::new();
    for fork in forks {
        let fork_owner = db::user::get_by_id(&state.db, &fork.owner)
            .await
            .map(|u| u.username)
            .unwrap_or_else(|_| fork.owner.clone());
        responses.push(ForkResponse::from_repo(fork, &fork_owner));
    }

    Ok(Json(responses))
}
