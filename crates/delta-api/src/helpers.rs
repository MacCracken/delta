//! Shared helper functions for route handlers.

use axum::http::StatusCode;
use delta_core::db;
use delta_core::models::repo::{Repository, Visibility};
use delta_core::models::user::User;

use crate::state::AppState;

/// Resolve a repository by owner username and repo name.
/// Returns (Repository, owner User).
///
/// **Important**: For endpoints that accept `AuthUser`, pass the user to
/// `resolve_repo_authed` instead, which enforces private repo visibility.
pub async fn resolve_repo(
    state: &AppState,
    owner: &str,
    name: &str,
) -> Result<(Repository, User), (StatusCode, String)> {
    let owner_user = db::user::get_by_username(&state.db, owner)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "user not found".into()))?;
    let owner_id = owner_user.id.to_string();
    let repo = db::repo::get_by_owner_and_name(&state.db, &owner_id, name)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "repository not found".into()))?;

    // Block access to non-public repos when called without a user context.
    // Endpoints with AuthUser should use resolve_repo_authed instead.
    if repo.visibility != Visibility::Public {
        return Err((StatusCode::NOT_FOUND, "repository not found".into()));
    }

    Ok((repo, owner_user))
}

/// Resolve a repository and enforce visibility for the authenticated user.
/// Private repos are only visible to the owner.
pub async fn resolve_repo_authed(
    state: &AppState,
    owner: &str,
    name: &str,
    user: &User,
) -> Result<(Repository, User), (StatusCode, String)> {
    let owner_user = db::user::get_by_username(&state.db, owner)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "user not found".into()))?;
    let owner_id = owner_user.id.to_string();
    let repo = db::repo::get_by_owner_and_name(&state.db, &owner_id, name)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "repository not found".into()))?;

    if repo.visibility != Visibility::Public && user.id != owner_user.id {
        return Err((StatusCode::NOT_FOUND, "repository not found".into()));
    }

    Ok((repo, owner_user))
}
