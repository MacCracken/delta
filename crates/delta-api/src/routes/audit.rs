//! Phase 7/8: Audit log API routes.

use axum::{
    Json, Router,
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
};
use delta_core::db;
use serde::Deserialize;

use crate::extractors::AuthUser;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_audit_log))
        .route("/export", get(export_audit_logs))
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
    .map_err(|e| {
        tracing::error!("failed to list audit log: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal server error".into(),
        )
    })?;
    Ok(Json(entries))
}

#[derive(Deserialize)]
struct ExportQuery {
    since: Option<String>,
    until: Option<String>,
    resource_type: Option<String>,
    #[serde(default = "default_export_limit")]
    limit: i64,
    #[serde(default)]
    offset: i64,
    /// Output format: "json" (default) or "csv"
    #[serde(default = "default_format")]
    format: String,
}

fn default_export_limit() -> i64 {
    10000
}

fn default_format() -> String {
    "json".into()
}

async fn export_audit_logs(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Query(params): Query<ExportQuery>,
) -> Result<axum::response::Response, (StatusCode, String)> {
    let limit = params.limit.min(100000);

    let entries = db::audit::list_for_export(
        &state.db,
        params.since.as_deref(),
        params.until.as_deref(),
        params.resource_type.as_deref(),
        limit,
        params.offset,
    )
    .await
    .map_err(|e| {
        tracing::error!("audit export failed: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal server error".into(),
        )
    })?;

    let total = db::audit::count_for_export(
        &state.db,
        params.since.as_deref(),
        params.until.as_deref(),
        params.resource_type.as_deref(),
    )
    .await
    .unwrap_or(0);

    // Compute BLAKE3 integrity hash over the export data
    let json_bytes = serde_json::to_vec(&entries).unwrap_or_default();
    let integrity_hash = blake3::hash(&json_bytes).to_hex().to_string();

    if params.format == "csv" {
        let mut csv = String::from(
            "id,user_id,action,resource_type,resource_id,details,ip_address,created_at\n",
        );
        for e in &entries {
            csv.push_str(&format!(
                "{},{},{},{},{},{},{},{}\n",
                csv_escape(&e.id),
                csv_escape(e.user_id.as_deref().unwrap_or("")),
                csv_escape(&e.action),
                csv_escape(&e.resource_type),
                csv_escape(e.resource_id.as_deref().unwrap_or("")),
                csv_escape(e.details.as_deref().unwrap_or("")),
                csv_escape(e.ip_address.as_deref().unwrap_or("")),
                csv_escape(&e.created_at),
            ));
        }

        Ok((
            StatusCode::OK,
            [
                (
                    axum::http::header::CONTENT_TYPE,
                    "text/csv; charset=utf-8",
                ),
                (
                    axum::http::header::CONTENT_DISPOSITION,
                    "attachment; filename=\"audit_export.csv\"",
                ),
            ],
            csv,
        )
            .into_response())
    } else {
        let export = serde_json::json!({
            "entries": entries,
            "total": total,
            "limit": limit,
            "offset": params.offset,
            "integrity_hash": integrity_hash,
            "exported_at": chrono::Utc::now().to_rfc3339(),
        });

        Ok(Json(export).into_response())
    }
}

fn csv_escape(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}
