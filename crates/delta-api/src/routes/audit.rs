//! Phase 7/8: Audit log API routes.

use axum::{
    Json, Router,
    extract::{Query, State},
    http::StatusCode,
    routing::get,
};
use delta_core::db;
use serde::Deserialize;

use crate::extractors::AuthUser;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new().route("/", get(list_audit_log))
}

#[derive(Deserialize)]
struct AuditQuery {
    user_id: Option<String>,
    resource_type: Option<String>,
    #[serde(default = "default_limit")]
    limit: i64,
    #[serde(default)]
    offset: i64,
}
fn default_limit() -> i64 {
    100
}

async fn list_audit_log(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Query(query): Query<AuditQuery>,
) -> Result<Json<Vec<db::audit::AuditEntry>>, (StatusCode, String)> {
    let own_id = user.id.to_string();

    // Users can only view their own audit logs
    if let Some(ref requested_id) = query.user_id
        && *requested_id != own_id
    {
        return Err((
            StatusCode::FORBIDDEN,
            "cannot view another user's audit log".into(),
        ));
    }

    let limit = query.limit.clamp(1, 500);
    let entries = db::audit::list(
        &state.db,
        Some(&own_id),
        query.resource_type.as_deref(),
        limit,
        query.offset.max(0),
    )
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(entries))
}
