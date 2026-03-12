//! Shared helper functions for route handlers.

use axum::http::StatusCode;
use delta_core::db;
use delta_core::models::collaborator::CollaboratorRole;
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
/// Private repos are visible to the owner and collaborators.
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
        // Check if user is a collaborator (any role grants visibility)
        let is_collab =
            db::collaborator::get_role(&state.db, &repo.id.to_string(), &user.id.to_string())
                .await
                .unwrap_or(None)
                .is_some();

        if !is_collab {
            return Err((StatusCode::NOT_FOUND, "repository not found".into()));
        }
    }

    Ok((repo, owner_user))
}

/// Check whether the user is the repo owner or has the required collaborator role.
/// Returns Ok(()) if authorized, Err(403) otherwise.
pub async fn require_role(
    state: &AppState,
    repo: &Repository,
    owner_user: &User,
    user: &User,
    required: CollaboratorRole,
) -> Result<(), (StatusCode, String)> {
    // Owner always has full access
    if user.id == owner_user.id {
        return Ok(());
    }

    let role = db::collaborator::get_role(&state.db, &repo.id.to_string(), &user.id.to_string())
        .await
        .unwrap_or(None);

    match role {
        Some(r) if r.has(required) => Ok(()),
        _ => Err((StatusCode::FORBIDDEN, "insufficient permissions".into())),
    }
}
