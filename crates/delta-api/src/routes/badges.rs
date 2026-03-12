//! Pipeline status badge endpoint.
//!
//! Returns an SVG badge showing the latest pipeline status for a repository.
//! Public repos require no auth. Private repos return 404.

use axum::{
    Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
};
use delta_core::db;
use delta_core::db::pipeline::RunStatus;
use delta_core::models::repo::Visibility;
use serde::Deserialize;

use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new().route("/{owner}/{name}/badge.svg", get(pipeline_badge))
}

#[derive(Deserialize)]
struct BadgeQuery {
    branch: Option<String>,
}

async fn pipeline_badge(
    State(state): State<AppState>,
    Path((owner, name)): Path<(String, String)>,
    Query(query): Query<BadgeQuery>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let owner_user = db::user::get_by_username(&state.db, &owner)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "not found".into()))?;

    let owner_id = owner_user.id.to_string();
    let repo = db::repo::get_by_owner_and_name(&state.db, &owner_id, &name)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "not found".into()))?;

    // Only public repos expose badges without auth
    if repo.visibility != Visibility::Public {
        return Err((StatusCode::NOT_FOUND, "not found".into()));
    }

    let latest = db::pipeline::get_latest(
        &state.db,
        &repo.id.to_string(),
        query.branch.as_deref(),
    )
    .await
    .map_err(|e| {
        tracing::error!("failed to get latest pipeline: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal server error".into(),
        )
    })?;

    let (status_text, color) = match latest.as_ref().map(|r| r.status) {
        Some(RunStatus::Passed) => ("passing", "#4c1"),
        Some(RunStatus::Failed) => ("failing", "#e05d44"),
        Some(RunStatus::Running) => ("running", "#dfb317"),
        Some(RunStatus::Queued) => ("queued", "#9f9f9f"),
        Some(RunStatus::Cancelled) => ("cancelled", "#9f9f9f"),
        None => ("unknown", "#9f9f9f"),
    };

    let svg = render_badge("pipeline", status_text, color);

    Ok((
        StatusCode::OK,
        [
            (
                axum::http::header::CONTENT_TYPE,
                "image/svg+xml".to_string(),
            ),
            (
                axum::http::header::CACHE_CONTROL,
                "no-cache, max-age=0".to_string(),
            ),
        ],
        svg,
    ))
}

fn render_badge(label: &str, status: &str, color: &str) -> String {
    let label_width = label.len() as u32 * 7 + 10;
    let status_width = status.len() as u32 * 7 + 10;
    let total_width = label_width + status_width;
    let label_x = label_width as f32 / 2.0;
    let status_x = label_width as f32 + status_width as f32 / 2.0;

    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{total_width}" height="20" role="img" aria-label="{label}: {status}">
  <title>{label}: {status}</title>
  <linearGradient id="s" x2="0" y2="100%">
    <stop offset="0" stop-color="#bbb" stop-opacity=".1"/>
    <stop offset="1" stop-opacity=".1"/>
  </linearGradient>
  <clipPath id="r"><rect width="{total_width}" height="20" rx="3" fill="#fff"/></clipPath>
  <g clip-path="url(#r)">
    <rect width="{label_width}" height="20" fill="#555"/>
    <rect x="{label_width}" width="{status_width}" height="20" fill="{color}"/>
    <rect width="{total_width}" height="20" fill="url(#s)"/>
  </g>
  <g fill="#fff" text-anchor="middle" font-family="Verdana,Geneva,DejaVu Sans,sans-serif" text-rendering="geometricPrecision" font-size="11">
    <text x="{label_x}" y="14">{label}</text>
    <text x="{status_x}" y="14">{status}</text>
  </g>
</svg>"##
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_badge_passing() {
        let svg = render_badge("pipeline", "passing", "#4c1");
        assert!(svg.contains("<svg"));
        assert!(svg.contains("pipeline"));
        assert!(svg.contains("passing"));
        assert!(svg.contains("#4c1"));
    }

    #[test]
    fn test_render_badge_failing() {
        let svg = render_badge("pipeline", "failing", "#e05d44");
        assert!(svg.contains("failing"));
        assert!(svg.contains("#e05d44"));
    }

    #[test]
    fn test_render_badge_dimensions() {
        let svg = render_badge("ci", "ok", "#4c1");
        // "ci" = 2 chars -> 2*7+10 = 24
        // "ok" = 2 chars -> 2*7+10 = 24
        assert!(svg.contains(r#"width="48""#));
    }
}
