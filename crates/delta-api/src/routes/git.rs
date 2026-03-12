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
    // Axum allows only one parameter per path segment, so we capture
    // the full "{name}.git" as {repo} and strip the suffix in a helper.
    //
    // NOTE: These routes are merged at the top level (no prefix). The {repo}
    // segment is expected to end in ".git" (e.g. "myrepo.git") which prevents
    // collisions with the /api/v1/ and /health prefixed routes.
    Router::new()
        .route("/{owner}/{repo}/info/refs", get(info_refs))
        .route("/{owner}/{repo}/git-upload-pack", post(upload_pack))
        .route("/{owner}/{repo}/git-receive-pack", post(receive_pack))
}

/// Strip the `.git` suffix from a repo path segment (e.g. "myrepo.git" → "myrepo").
fn parse_repo_name(repo: &str) -> &str {
    repo.strip_suffix(".git").unwrap_or(repo)
}

#[derive(Deserialize)]
struct InfoRefsQuery {
    service: String,
}

async fn info_refs(
    State(state): State<AppState>,
    Path((owner, repo)): Path<(String, String)>,
    Query(query): Query<InfoRefsQuery>,
    headers: HeaderMap,
) -> Result<Response, (StatusCode, String)> {
    let name = parse_repo_name(&repo);
    // Check repo exists
    let repo_path = state
        .repo_host
        .repo_path(&owner, name)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    if !repo_path.exists() {
        return Err((StatusCode::NOT_FOUND, "repository not found".into()));
    }

    // For receive-pack (push), require authentication
    if query.service == "git-receive-pack" {
        authenticate_git_request(&state, &headers, &owner, name)
            .await
            .map_err(|e| (StatusCode::UNAUTHORIZED, e))?;
    } else {
        // For upload-pack (clone/fetch), check visibility
        check_read_access(&state, &owner, name, &headers).await?;
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
    Path((owner, repo)): Path<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, (StatusCode, String)> {
    let name = parse_repo_name(&repo);
    let repo_path = state
        .repo_host
        .repo_path(&owner, name)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    if !repo_path.exists() {
        return Err((StatusCode::NOT_FOUND, "repository not found".into()));
    }

    check_read_access(&state, &owner, name, &headers).await?;

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
    Path((owner, repo)): Path<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, (StatusCode, String)> {
    let name = parse_repo_name(&repo);
    let repo_path = state
        .repo_host
        .repo_path(&owner, name)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    if !repo_path.exists() {
        return Err((StatusCode::NOT_FOUND, "repository not found".into()));
    }

    // Push always requires auth
    let user = authenticate_git_request(&state, &headers, &owner, name)
        .await
        .map_err(|e| (StatusCode::UNAUTHORIZED, e))?;

    let output = delta_vcs::protocol::receive_pack(&repo_path, &body)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Fire webhooks and trigger pipelines asynchronously
    let db = state.db.clone();
    let owner_clone = owner.clone();
    let name_clone = name.to_owned();
    let pusher = user;
    let repo_path_clone = repo_path.clone();
    let secrets_key = state.config.auth.secrets_key.clone();
    let webhooks_https_only = state.config.webhooks.https_only;
    tokio::spawn(async move {
        if let Err(e) =
            dispatch_push_webhooks(&db, &owner_clone, &name_clone, &pusher, webhooks_https_only)
                .await
        {
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
/// For push operations, verifies the user is the owner or has write+ collaborator access.
async fn authenticate_git_request(
    state: &AppState,
    headers: &HeaderMap,
    expected_owner: &str,
    repo_name: &str,
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

    // Owner always has push access
    if user.username != expected_owner {
        // Check collaborator write access
        let owner_user = delta_core::db::user::get_by_username(&state.db, expected_owner)
            .await
            .map_err(|_| "repository owner not found".to_string())?;
        let owner_id = owner_user.id.to_string();
        let repo = delta_core::db::repo::get_by_owner_and_name(&state.db, &owner_id, repo_name)
            .await
            .map_err(|_| "repository not found".to_string())?;
        let role = delta_core::db::collaborator::get_role(
            &state.db,
            &repo.id.to_string(),
            &user.id.to_string(),
        )
        .await
        .unwrap_or(None);
        match role {
            Some(r) if r.has(delta_core::models::collaborator::CollaboratorRole::Write) => {
                // Collaborator with write access — allowed
            }
            _ => {
                return Err("you don't have push access to this repository".to_string());
            }
        }
    }

    Ok(user.username)
}

/// Check read access — public repos are open, private repos need owner or collaborator auth.
async fn check_read_access(
    state: &AppState,
    owner: &str,
    name: &str,
    headers: &HeaderMap,
) -> std::result::Result<(), (StatusCode, String)> {
    // Look up repo visibility
    let owner_user = delta_core::db::user::get_by_username(&state.db, owner).await;

    if let Ok(ref owner_user) = owner_user {
        let owner_id = owner_user.id.to_string();
        if let Ok(repo) =
            delta_core::db::repo::get_by_owner_and_name(&state.db, &owner_id, name).await
        {
            if repo.visibility == delta_core::models::repo::Visibility::Public {
                return Ok(());
            }
            // Private repo — try to authenticate and check access
            let auth_result = authenticate_git_user(state, headers).await;
            match auth_result {
                Ok(user) => {
                    if user.username == owner {
                        return Ok(());
                    }
                    // Check collaborator read access
                    let role = delta_core::db::collaborator::get_role(
                        &state.db,
                        &repo.id.to_string(),
                        &user.id.to_string(),
                    )
                    .await
                    .unwrap_or(None);
                    if role.is_some() {
                        return Ok(());
                    }
                    return Err((StatusCode::NOT_FOUND, "repository not found".into()));
                }
                Err(e) => return Err((StatusCode::UNAUTHORIZED, e)),
            }
        }
    }

    Err((StatusCode::NOT_FOUND, "repository not found".into()))
}

/// Authenticate a git HTTP request (Basic auth) and return the User.
/// Does NOT check repository-level permissions.
async fn authenticate_git_user(
    state: &AppState,
    headers: &HeaderMap,
) -> std::result::Result<delta_core::models::user::User, String> {
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

    let (username, token) = decoded_str
        .split_once(':')
        .ok_or("invalid credential format")?;

    let user = crate::auth::authenticate_token(&state.db, token)
        .await
        .map_err(|_| "invalid or expired token".to_string())?;

    if user.username != username {
        return Err("username mismatch".to_string());
    }

    Ok(user)
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
    let h = host.trim_start_matches('[').trim_end_matches(']');
    h == "localhost"
        || h == "127.0.0.1"
        || h == "::1"
        || h == "0.0.0.0"
        || h.starts_with("10.")
        || h.starts_with("192.168.")
        || h.starts_with("169.254.")
        || h.starts_with("fe80:")
        || h.starts_with("fc00:")
        || h.starts_with("fd")
        || is_private_172(h)
        || h.ends_with(".local")
        || h.ends_with(".internal")
}

/// Dispatch push webhooks for a repository.
async fn dispatch_push_webhooks(
    db: &sqlx::SqlitePool,
    owner: &str,
    name: &str,
    pusher: &str,
    https_only: bool,
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
        if https_only && !webhook.url.starts_with("https://") {
            tracing::warn!(webhook_id = %webhook.id, "skipping non-HTTPS webhook (https_only enabled)");
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_repo_name_with_git_suffix() {
        assert_eq!(parse_repo_name("myrepo.git"), "myrepo");
    }

    #[test]
    fn test_parse_repo_name_without_suffix() {
        assert_eq!(parse_repo_name("myrepo"), "myrepo");
    }

    #[test]
    fn test_parse_repo_name_double_git() {
        assert_eq!(parse_repo_name("my.git.git"), "my.git");
    }

    #[test]
    fn test_is_private_url_localhost() {
        assert!(is_private_url("http://localhost/webhook"));
        assert!(is_private_url("http://localhost:8080/hook"));
    }
    #[test]
    fn test_is_private_url_loopback() {
        assert!(is_private_url("http://127.0.0.1/hook"));
        assert!(is_private_url("http://127.0.0.1:3000/hook"));
    }
    #[test]
    fn test_is_private_url_ipv6_loopback() {
        assert!(is_private_url("http://[::1]/hook"));
    }
    #[test]
    fn test_is_private_url_zero_addr() {
        assert!(is_private_url("http://0.0.0.0/hook"));
    }
    #[test]
    fn test_is_private_url_rfc1918() {
        assert!(is_private_url("http://10.0.0.1/hook"));
        assert!(is_private_url("http://10.255.255.255/hook"));
        assert!(is_private_url("http://192.168.1.1/hook"));
        assert!(is_private_url("http://192.168.0.100/hook"));
    }
    #[test]
    fn test_is_private_url_172_range() {
        assert!(is_private_url("http://172.16.0.1/hook"));
        assert!(is_private_url("http://172.31.255.255/hook"));
        assert!(!is_private_url("http://172.15.0.1/hook"));
        assert!(!is_private_url("http://172.32.0.1/hook"));
    }
    #[test]
    fn test_is_private_url_link_local() {
        assert!(is_private_url("http://169.254.1.1/hook"));
    }
    #[test]
    fn test_is_private_url_ipv6_private() {
        assert!(is_private_url("http://[fe80::1]/hook"));
        assert!(is_private_url("http://[fc00::1]/hook"));
        assert!(is_private_url("http://[fd12::1]/hook"));
    }
    #[test]
    fn test_is_private_url_mdns_and_internal() {
        assert!(is_private_url("http://myhost.local/hook"));
        assert!(is_private_url("http://service.internal/hook"));
    }
    #[test]
    fn test_is_private_url_public() {
        assert!(!is_private_url("https://example.com/hook"));
        assert!(!is_private_url("https://api.github.com/webhook"));
        assert!(!is_private_url("http://8.8.8.8/hook"));
    }
    #[test]
    fn test_is_private_url_unparseable() {
        assert!(is_private_url("not-a-url"));
        assert!(is_private_url(""));
    }
    #[test]
    fn test_is_private_172_edge_cases() {
        assert!(is_private_172("172.16.0.1"));
        assert!(is_private_172("172.31.255.255"));
        assert!(!is_private_172("172.15.0.1"));
        assert!(!is_private_172("172.32.0.1"));
        assert!(!is_private_172("173.16.0.1"));
        assert!(!is_private_172("172.abc.0.1"));
        assert!(!is_private_172("not-an-ip"));
    }
}
