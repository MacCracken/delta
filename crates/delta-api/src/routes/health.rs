use axum::{Router, routing::get};
use serde_json::json;

pub fn router() -> Router<crate::state::AppState> {
    Router::new().route("/", get(health_check))
}

async fn health_check() -> axum::Json<serde_json::Value> {
    axum::Json(json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}
