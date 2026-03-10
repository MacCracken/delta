pub mod health;
pub mod repos;

use axum::Router;
use crate::state::AppState;

pub fn router(state: AppState) -> Router {
    Router::new()
        .nest("/health", health::router())
        .nest("/api/v1/repos", repos::router())
        .with_state(state)
}
