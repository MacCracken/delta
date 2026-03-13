//! Health check and metrics endpoints.

use axum::{Json, Router, extract::State, routing::get};
use serde_json::json;

use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(health_check))
        .route("/ready", get(readiness_check))
        .route("/metrics", get(metrics))
}

async fn health_check() -> Json<serde_json::Value> {
    Json(json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

async fn readiness_check(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, (axum::http::StatusCode, String)> {
    // Check database connectivity
    let db_ok = sqlx::query_scalar::<_, i64>("SELECT 1")
        .fetch_one(&state.db)
        .await
        .is_ok();

    // Check storage directories
    let repos_ok = state.config.storage.repos_dir.exists();
    let artifacts_ok = state.config.storage.artifacts_dir.exists();

    let ready = db_ok && repos_ok && artifacts_ok;

    let body = json!({
        "ready": ready,
        "checks": {
            "database": if db_ok { "ok" } else { "error" },
            "repos_storage": if repos_ok { "ok" } else { "error" },
            "artifacts_storage": if artifacts_ok { "ok" } else { "error" },
        },
        "version": env!("CARGO_PKG_VERSION"),
    });

    if ready {
        Ok(Json(body))
    } else {
        Err((
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            serde_json::to_string(&body).unwrap_or_default(),
        ))
    }
}

async fn metrics(State(state): State<AppState>) -> Json<serde_json::Value> {
    use std::sync::atomic::Ordering;

    let m = &state.metrics;
    let total = m.total_requests.load(Ordering::Relaxed);
    let duration_us = m.total_duration_us.load(Ordering::Relaxed);
    let uptime = m.started_at.elapsed().as_secs();

    let mut status_counts = serde_json::Map::new();
    for entry in m.request_counts.iter() {
        status_counts.insert(entry.key().to_string(), json!(*entry.value()));
    }

    let avg_latency_us = if total > 0 { duration_us / total } else { 0 };

    Json(json!({
        "uptime_secs": uptime,
        "total_requests": total,
        "avg_latency_us": avg_latency_us,
        "status_codes": status_counts,
        "rate_limiting": {
            "enabled": state.rate_limiter.is_some(),
        },
        "version": env!("CARGO_PKG_VERSION"),
    }))
}
