//! Shared helper functions for route handlers.

use axum::http::StatusCode;
use delta_core::db;
use delta_core::models::{repo::Repository, user::User};

use crate::state::AppState;

/// Resolve a repository by owner username and repo name.
/// Returns (Repository, owner User).
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
    Ok((repo, owner_user))
}
