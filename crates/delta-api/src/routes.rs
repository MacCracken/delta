pub mod auth;
pub mod branches;
pub mod git;
pub mod health;
pub mod pulls;
pub mod repos;
pub mod status_checks;
pub mod webhooks;

use axum::Router;
use crate::state::AppState;

pub fn router(state: AppState) -> Router {
    Router::new()
        .nest("/health", health::router())
        .nest("/api/v1/auth", auth::router())
        .nest("/api/v1/repos", repos::router())
        .nest("/api/v1/repos", branches::router())
        .nest("/api/v1/repos", webhooks::router())
        .nest("/api/v1/repos", pulls::router())
        .nest("/api/v1/repos", status_checks::router())
        // Git smart HTTP — no prefix, matches /{owner}/{name}.git/...
        .merge(git::router())
        .with_state(state)
}
