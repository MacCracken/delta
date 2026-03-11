pub mod artifacts;
pub mod audit;
pub mod auth;
pub mod branches;
pub mod git;
pub mod health;
pub mod pipelines;
pub mod pulls;
pub mod repos;
pub mod status_checks;
pub mod webhooks;

use crate::state::AppState;
use axum::Router;
use tower_http::cors::{Any, CorsLayer};

pub fn router(state: AppState) -> Router {
    let cors = {
        let base = CorsLayer::new()
            .allow_methods([
                axum::http::Method::GET,
                axum::http::Method::POST,
                axum::http::Method::PUT,
                axum::http::Method::PATCH,
                axum::http::Method::DELETE,
            ])
            .allow_headers(Any);

        if state.config.server.cors_origins.is_empty() {
            tracing::warn!("CORS: allow_origin(Any) — set server.cors_origins in production");
            base.allow_origin(Any)
        } else {
            let origins: Vec<axum::http::HeaderValue> = state
                .config
                .server
                .cors_origins
                .iter()
                .filter_map(|o| o.parse().ok())
                .collect();
            base.allow_origin(origins)
        }
    };

    Router::new()
        .nest("/health", health::router())
        .nest("/api/v1/auth", auth::router())
        .nest("/api/v1/repos", repos::router())
        .nest("/api/v1/repos", branches::router())
        .nest("/api/v1/repos", webhooks::router())
        .nest("/api/v1/repos", pulls::router())
        .nest("/api/v1/repos", status_checks::router())
        .nest("/api/v1/repos", pipelines::router())
        .nest("/api/v1/repos", artifacts::router())
        .nest("/api/v1/audit", audit::router())
        // Git smart HTTP — no prefix, matches /{owner}/{name}.git/...
        .merge(git::router())
        .layer(cors)
        .with_state(state)
}
