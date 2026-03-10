//! Git smart HTTP transport routes.
//!
//! These routes implement the git smart HTTP protocol, enabling standard
//! git clients to clone, fetch, and push via HTTP.
//!
//! Routes:
//!   GET  /{owner}/{name}.git/info/refs?service=git-upload-pack
//!   GET  /{owner}/{name}.git/info/refs?service=git-receive-pack
//!   POST /{owner}/{name}.git/git-upload-pack
//!   POST /{owner}/{name}.git/git-receive-pack

use axum::{
    Router,
    body::Bytes,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::Deserialize;

use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/{owner}/{name}.git/info/refs", get(info_refs))
        .route("/{owner}/{name}.git/git-upload-pack", post(upload_pack))
        .route("/{owner}/{name}.git/git-receive-pack", post(receive_pack))
}

#[derive(Deserialize)]
struct InfoRefsQuery {
    service: String,
}

async fn info_refs(
    State(state): State<AppState>,
    Path((owner, name)): Path<(String, String)>,
    Query(query): Query<InfoRefsQuery>,
    headers: HeaderMap,
) -> Result<Response, (StatusCode, String)> {
    // Check repo exists
    let repo_path = state.repo_host.repo_path(&owner, &name);
    if !repo_path.exists() {
        return Err((StatusCode::NOT_FOUND, "repository not found".into()));
    }

    // For receive-pack (push), require authentication
    if query.service == "git-receive-pack" {
        authenticate_git_request(&state, &headers, &owner)
            .await
            .map_err(|e| (StatusCode::UNAUTHORIZED, e))?;
    } else {
        // For upload-pack (clone/fetch), check visibility
        check_read_access(&state, &owner, &name, &headers).await?;
    }

    let body = delta_vcs::protocol::advertise_refs(&repo_path, &query.service)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let content_type = format!("application/x-{}-advertisement", query.service);
    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, "no-cache".into()),
        ],
        body,
    )
        .into_response())
}

async fn upload_pack(
    State(state): State<AppState>,
    Path((owner, name)): Path<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, (StatusCode, String)> {
    let repo_path = state.repo_host.repo_path(&owner, &name);
    if !repo_path.exists() {
        return Err((StatusCode::NOT_FOUND, "repository not found".into()));
    }

    check_read_access(&state, &owner, &name, &headers).await?;

    let output = delta_vcs::protocol::upload_pack(&repo_path, &body)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok((
        StatusCode::OK,
        [(
            header::CONTENT_TYPE,
            "application/x-git-upload-pack-result".to_string(),
        )],
        output,
    )
        .into_response())
}

async fn receive_pack(
    State(state): State<AppState>,
    Path((owner, name)): Path<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, (StatusCode, String)> {
    let repo_path = state.repo_host.repo_path(&owner, &name);
    if !repo_path.exists() {
        return Err((StatusCode::NOT_FOUND, "repository not found".into()));
    }

    // Push always requires auth
    let user = authenticate_git_request(&state, &headers, &owner)
        .await
        .map_err(|e| (StatusCode::UNAUTHORIZED, e))?;

    let output = delta_vcs::protocol::receive_pack(&repo_path, &body)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Fire webhooks asynchronously
    let db = state.db.clone();
    let owner_clone = owner.clone();
    let name_clone = name.clone();
    let pusher = user;
    tokio::spawn(async move {
        if let Err(e) = dispatch_push_webhooks(&db, &owner_clone, &name_clone, &pusher).await {
            tracing::warn!("webhook dispatch failed: {}", e);
        }
    });

    Ok((
        StatusCode::OK,
        [(
            header::CONTENT_TYPE,
            "application/x-git-receive-pack-result".to_string(),
        )],
        output,
    )
        .into_response())
}

/// Authenticate a git HTTP request using Basic auth (username + token).
async fn authenticate_git_request(
    state: &AppState,
    headers: &HeaderMap,
    expected_owner: &str,
) -> std::result::Result<String, String> {
    let auth_header = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .ok_or("authentication required")?;

    let credentials = auth_header
        .strip_prefix("Basic ")
        .ok_or("invalid auth format — use Basic auth with token as password")?;

    let decoded = base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        credentials,
    )
    .map_err(|_| "invalid base64 credentials".to_string())?;

    let decoded_str =
        String::from_utf8(decoded).map_err(|_| "invalid utf-8 credentials".to_string())?;

    // Format: username:token
    let (username, token) = decoded_str
        .split_once(':')
        .ok_or("invalid credential format")?;

    let user = crate::auth::authenticate_token(&state.db, token)
        .await
        .map_err(|_| "invalid or expired token".to_string())?;

    if user.username != username {
        return Err("username mismatch".to_string());
    }

    // For push, verify ownership
    if user.username != expected_owner {
        // TODO: check collaborator access
        return Err("you don't have push access to this repository".to_string());
    }

    Ok(user.username)
}

/// Check read access — public repos are open, private repos need auth.
async fn check_read_access(
    state: &AppState,
    owner: &str,
    name: &str,
    headers: &HeaderMap,
) -> std::result::Result<(), (StatusCode, String)> {
    // Look up repo visibility
    let owner_user = delta_core::db::user::get_by_username(&state.db, owner).await;

    if let Ok(owner_user) = owner_user {
        let owner_id = owner_user.id.to_string();
        if let Ok(repo) = delta_core::db::repo::get_by_owner_and_name(&state.db, &owner_id, name).await
            && repo.visibility == delta_core::models::repo::Visibility::Public
        {
            return Ok(());
        }
    }

    // Private or unknown — require auth
    authenticate_git_request(state, headers, owner)
        .await
        .map(|_| ())
        .map_err(|e| (StatusCode::UNAUTHORIZED, e))
}

/// Dispatch push webhooks for a repository.
async fn dispatch_push_webhooks(
    db: &sqlx::SqlitePool,
    owner: &str,
    name: &str,
    pusher: &str,
) -> delta_core::Result<()> {
    let owner_user = delta_core::db::user::get_by_username(db, owner).await?;
    let owner_id = owner_user.id.to_string();
    let repo = delta_core::db::repo::get_by_owner_and_name(db, &owner_id, name).await?;
    let repo_id = repo.id.to_string();

    let webhooks = delta_core::db::webhook::get_for_event(db, &repo_id, "push").await?;
    if webhooks.is_empty() {
        return Ok(());
    }

    let payload = serde_json::json!({
        "event": "push",
        "repo_owner": owner,
        "repo_name": name,
        "pusher": pusher,
        "timestamp": chrono::Utc::now().to_rfc3339(),
    });
    let payload_str = serde_json::to_string(&payload)?;

    let client = reqwest::Client::new();
    for webhook in webhooks {
        let resp = client
            .post(&webhook.url)
            .header("Content-Type", "application/json")
            .header("X-Delta-Event", "push")
            .body(payload_str.clone())
            .send()
            .await;

        let (status, body) = match resp {
            Ok(r) => {
                let status = r.status().as_u16() as i32;
                let body = r.text().await.unwrap_or_default();
                (Some(status), Some(body))
            }
            Err(e) => {
                tracing::warn!(webhook_id = %webhook.id, "webhook delivery failed: {}", e);
                (None, Some(e.to_string()))
            }
        };

        let _ = delta_core::db::webhook::record_delivery(
            db,
            &webhook.id,
            "push",
            &payload_str,
            status,
            body.as_deref(),
        )
        .await;
    }

    Ok(())
}
