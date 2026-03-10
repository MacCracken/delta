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
    let repo_path = state
        .repo_host
        .repo_path(&owner, &name)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
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
    let repo_path = state
        .repo_host
        .repo_path(&owner, &name)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
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
    let repo_path = state
        .repo_host
        .repo_path(&owner, &name)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
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

    // Fire webhooks and trigger pipelines asynchronously
    let db = state.db.clone();
    let owner_clone = owner.clone();
    let name_clone = name.clone();
    let pusher = user;
    let repo_path_clone = repo_path.clone();
    let secrets_key = state.config.auth.secrets_key.clone();
    tokio::spawn(async move {
        if let Err(e) = dispatch_push_webhooks(&db, &owner_clone, &name_clone, &pusher).await {
            tracing::warn!("webhook dispatch failed: {}", e);
        }
        if let Err(e) = dispatch_push_pipelines(
            &db,
            &owner_clone,
            &name_clone,
            &repo_path_clone,
            &secrets_key,
        )
        .await
        {
            tracing::warn!("pipeline dispatch failed: {}", e);
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

    let decoded = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, credentials)
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
        if let Ok(repo) =
            delta_core::db::repo::get_by_owner_and_name(&state.db, &owner_id, name).await
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

/// Check if a host is in the 172.16.0.0/12 private range (172.16.x - 172.31.x).
fn is_private_172(host: &str) -> bool {
    if let Some(rest) = host.strip_prefix("172.")
        && let Some(octet_str) = rest.split('.').next()
        && let Ok(octet) = octet_str.parse::<u8>()
    {
        return (16..=31).contains(&octet);
    }
    false
}

/// Check if a URL targets a private/internal network (SSRF protection).
pub fn is_private_url(url_str: &str) -> bool {
    let Ok(url) = url::Url::parse(url_str) else {
        return true; // Reject unparseable URLs
    };
    let Some(host) = url.host_str() else {
        return true;
    };
    host == "localhost"
        || host == "127.0.0.1"
        || host == "::1"
        || host == "[::1]"
        || host == "0.0.0.0"
        || host.starts_with("10.")
        || host.starts_with("192.168.")
        || host.starts_with("169.254.")
        || host.starts_with("fe80:")
        || host.starts_with("fc00:")
        || host.starts_with("fd")
        || is_private_172(host)
        || host.ends_with(".local")
        || host.ends_with(".internal")
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

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| delta_core::DeltaError::Storage(format!("HTTP client error: {e}")))?;

    for webhook in webhooks {
        // Validate webhook URL: must be HTTP(S) and not target private networks
        if !webhook.url.starts_with("https://") && !webhook.url.starts_with("http://") {
            tracing::warn!(webhook_id = %webhook.id, "skipping webhook with non-HTTP URL");
            continue;
        }
        if is_private_url(&webhook.url) {
            tracing::warn!(webhook_id = %webhook.id, "skipping webhook targeting private network");
            continue;
        }

        // Compute HMAC signature if webhook has a secret
        let signature = webhook.secret.as_deref().map(|secret| {
            use blake3::Hasher;
            let mut hasher = Hasher::new_keyed(&blake3::hash(secret.as_bytes()).as_bytes().clone());
            hasher.update(payload_str.as_bytes());
            hasher.finalize().to_hex().to_string()
        });

        let mut req_builder = client
            .post(&webhook.url)
            .header("Content-Type", "application/json")
            .header("X-Delta-Event", "push");

        if let Some(sig) = &signature {
            req_builder = req_builder.header("X-Delta-Signature", sig.as_str());
        }

        let resp = req_builder.body(payload_str.clone()).send().await;

        let (status, body) = match resp {
            Ok(r) => {
                let status = r.status().as_u16() as i32;
                let body = r.text().await.unwrap_or_else(|e| e.to_string());
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

/// Trigger CI/CD pipelines for a push event.
async fn dispatch_push_pipelines(
    db: &sqlx::SqlitePool,
    owner: &str,
    name: &str,
    repo_path: &std::path::Path,
    secrets_key: &str,
) -> std::result::Result<(), String> {
    // Resolve repo
    let owner_user = delta_core::db::user::get_by_username(db, owner)
        .await
        .map_err(|e| format!("failed to resolve owner: {e}"))?;
    let owner_id = owner_user.id.to_string();
    let repo = delta_core::db::repo::get_by_owner_and_name(db, &owner_id, name)
        .await
        .map_err(|e| format!("failed to resolve repo: {e}"))?;
    let repo_id = repo.id.to_string();

    // Get HEAD branch name
    let branch = delta_vcs::refs::head_branch(repo_path).unwrap_or_else(|| "main".into());

    // Get HEAD commit SHA
    let commit_sha = delta_vcs::refs::head_commit(repo_path)
        .ok()
        .flatten()
        .unwrap_or_default();

    if commit_sha.is_empty() {
        return Ok(());
    }

    // Decrypt repo secrets for pipeline env
    let encryption_key = delta_core::crypto::derive_key(secrets_key);
    let mut secrets = std::collections::HashMap::new();
    if let Ok(encrypted_secrets) = delta_core::db::secret::get_all_values(db, &repo_id).await {
        for (key, encrypted_value) in encrypted_secrets {
            if let Ok(value) = delta_core::crypto::decrypt(&encryption_key, &encrypted_value) {
                secrets.insert(key, value);
            }
        }
    }

    let ctx = delta_ci::runner::PipelineContext {
        pool: db,
        repo_id: &repo_id,
        repo_path,
        commit_sha: &commit_sha,
        secrets: &secrets,
    };
    delta_ci::runner::run_push_pipelines(&ctx, &branch).await;
    Ok(())
}
