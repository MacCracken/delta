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
//!
//! Request bodies are streamed into git and git's output is streamed back,
//! so neither packs nor clones are buffered in memory.

use std::time::Duration;

use axum::{
    Router,
    body::Body,
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use delta_core::models::repo::{Repository, Visibility};
use delta_core::models::user::User;
use delta_vcs::protocol::{self, RefUpdate};
use futures_util::TryStreamExt;
use serde::Deserialize;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::process::Child;
use tokio_util::io::{ReaderStream, StreamReader};

use crate::state::AppState;

/// Largest push accepted (the pack is streamed to git, not buffered).
const MAX_PUSH_BYTES: u64 = 2 * 1024 * 1024 * 1024;
/// Largest fetch negotiation request accepted.
const MAX_FETCH_REQUEST_BYTES: u64 = 64 * 1024 * 1024;
/// Largest command section (list of ref updates) accepted in a push.
const MAX_PUSH_COMMANDS_BYTES: usize = 1024 * 1024;
/// Upper bound on a single git transport process.
const RPC_TIMEOUT: Duration = Duration::from_secs(60 * 60);

type HttpError = (StatusCode, String);

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
        // git gzips fetch negotiation requests larger than 1 KiB.
        .layer(tower_http::decompression::RequestDecompressionLayer::new())
        .layer(axum::middleware::map_response(add_basic_auth_challenge))
}

/// git (via libcurl) only sends credentials after a 401 that names an
/// authentication scheme, so every 401 from these routes carries one.
async fn add_basic_auth_challenge(mut response: Response) -> Response {
    if response.status() == StatusCode::UNAUTHORIZED {
        response.headers_mut().insert(
            header::WWW_AUTHENTICATE,
            HeaderValue::from_static("Basic realm=\"Delta\""),
        );
    }
    response
}

/// Strip the `.git` suffix from a repo path segment (e.g. "myrepo.git" → "myrepo").
fn parse_repo_name(repo: &str) -> &str {
    crate::helpers::strip_git_suffix(repo)
}

fn internal_error(e: impl std::fmt::Display) -> HttpError {
    tracing::error!("git transport error: {}", e);
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        "internal server error".into(),
    )
}

/// On-disk path of an existing repository.
fn repo_dir(state: &AppState, owner: &str, name: &str) -> Result<std::path::PathBuf, HttpError> {
    let repo_path = state
        .repo_host
        .repo_path(owner, name)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    if !repo_path.exists() {
        return Err((StatusCode::NOT_FOUND, "repository not found".into()));
    }
    Ok(repo_path)
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
) -> Result<Response, HttpError> {
    let name = parse_repo_name(&repo);
    let repo_path = repo_dir(&state, &owner, name)?;

    match query.service.as_str() {
        "git-receive-pack" => {
            authorize_push(&state, &headers, &owner, name).await?;
        }
        "git-upload-pack" => check_read_access(&state, &owner, name, &headers).await?,
        _ => return Err((StatusCode::BAD_REQUEST, "unsupported git service".into())),
    }

    let body = protocol::advertise_refs(&repo_path, &query.service)
        .await
        .map_err(internal_error)?;

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

/// The request body as an `AsyncRead` (already gzip-decoded by the router).
fn body_reader(body: Body) -> impl AsyncRead + Unpin + Send + 'static {
    StreamReader::new(body.into_data_stream().map_err(std::io::Error::other))
}

/// Response carrying git's stdout for `service`.
fn rpc_result(service: &str, body: Body) -> Response {
    (
        StatusCode::OK,
        [
            (
                header::CONTENT_TYPE,
                format!("application/x-git-{service}-result"),
            ),
            (header::CACHE_CONTROL, "no-cache".to_string()),
        ],
        body,
    )
        .into_response()
}

/// Reap `child` in the background, killing it if it outlives `RPC_TIMEOUT`.
fn reap(service: &'static str, mut child: Child) {
    tokio::spawn(async move {
        match tokio::time::timeout(RPC_TIMEOUT, child.wait()).await {
            Ok(Ok(status)) if !status.success() => {
                tracing::debug!(service, %status, "git service exited with error");
            }
            Ok(Err(e)) => tracing::warn!(service, "failed to wait for git: {}", e),
            Err(_) => {
                tracing::warn!(service, "git service timed out; killing it");
                let _ = child.kill().await;
            }
            Ok(Ok(_)) => {}
        }
    });
}

async fn upload_pack(
    State(state): State<AppState>,
    Path((owner, repo)): Path<(String, String)>,
    headers: HeaderMap,
    body: Body,
) -> Result<Response, HttpError> {
    let name = parse_repo_name(&repo);
    let repo_path = repo_dir(&state, &owner, name)?;
    check_read_access(&state, &owner, name, &headers).await?;

    let mut child =
        protocol::spawn_service_rpc(&repo_path, "upload-pack", &[]).map_err(internal_error)?;
    let (Some(mut stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
        return Err(internal_error("git pipes unavailable"));
    };

    // Feed the request concurrently with streaming the response: git may
    // write more than a pipe buffer holds before it has read all input.
    tokio::spawn(async move {
        let mut limited = body_reader(body).take(MAX_FETCH_REQUEST_BYTES);
        if let Err(e) = tokio::io::copy(&mut limited, &mut stdin).await {
            tracing::debug!("upload-pack request stream ended: {}", e);
        }
        let _ = stdin.shutdown().await;
    });
    reap("upload-pack", child);

    Ok(rpc_result(
        "upload-pack",
        Body::from_stream(ReaderStream::new(stdout)),
    ))
}

async fn receive_pack(
    State(state): State<AppState>,
    Path((owner, repo)): Path<(String, String)>,
    headers: HeaderMap,
    body: Body,
) -> Result<Response, HttpError> {
    let name = parse_repo_name(&repo);
    let repo_path = repo_dir(&state, &owner, name)?;

    // Push always requires auth with write access.
    let (user, repo_record) = authorize_push(&state, &headers, &owner, name).await?;

    let mut output = Vec::new();
    serve_push(
        &state,
        &owner,
        &repo_record,
        &repo_path,
        &user,
        &mut body_reader(body),
        &mut output,
    )
    .await?;
    Ok(rpc_result("receive-pack", Body::from(output)))
}

/// Serve one push (the part after the ref advertisement) for either
/// transport: read the ref-update commands from `input`, enforce branch
/// protection, let git apply the commands and pack, and write git's report
/// to `output`. Webhooks and pipelines are dispatched for the updates git
/// applied.
pub(crate) async fn serve_push<R, W>(
    state: &AppState,
    owner: &str,
    repo: &Repository,
    repo_path: &std::path::Path,
    pusher: &User,
    input: &mut R,
    output: &mut W,
) -> Result<(), HttpError>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let bad_request = |e: &dyn std::fmt::Display| (StatusCode::BAD_REQUEST, e.to_string());

    // Read the ref-update commands before handing anything to git.
    let mut prefix = Vec::new();
    let commands_len = loop {
        if let Some(len) = protocol::receive_commands_len(&prefix).map_err(|e| bad_request(&e))? {
            break len;
        }
        if prefix.len() > MAX_PUSH_COMMANDS_BYTES {
            return Err((StatusCode::PAYLOAD_TOO_LARGE, "too many ref updates".into()));
        }
        let mut chunk = [0u8; 8192];
        let n = input.read(&mut chunk).await.map_err(|e| bad_request(&e))?;
        if n == 0 {
            if prefix.is_empty() {
                return Ok(()); // the client had nothing to send
            }
            return Err((StatusCode::BAD_REQUEST, "truncated push request".into()));
        }
        prefix.extend_from_slice(&chunk[..n]);
    };
    let commands =
        protocol::parse_receive_commands(&prefix[..commands_len]).map_err(|e| bad_request(&e))?;

    // Enforce branch protection. Fast-forward checks need the pushed
    // objects, so they are delegated to git via receive.denyNonFastForwards.
    let protections =
        delta_core::db::branch_protection::list_for_repo(&state.db, &repo.id.to_string())
            .await
            .map_err(internal_error)?;
    let mut fast_forward_only = false;
    for update in &commands.updates {
        let Some(branch) = update.refname.strip_prefix("refs/heads/") else {
            continue;
        };
        let Some(rule) = delta_core::models::branch_protection::BranchProtection::effective(
            &protections,
            branch,
        ) else {
            continue;
        };
        let violation = if rule.require_pr {
            Some("protected branch: changes must go through a pull request")
        } else if update.is_delete() && rule.prevent_deletion {
            Some("protected branch: deletion is not allowed")
        } else {
            None
        };
        if let Some(reason) = violation {
            output
                .write_all(&protocol::rejection_report(&commands, reason))
                .await
                .map_err(internal_error)?;
            return Ok(());
        }
        if rule.prevent_force_push && !update.is_create() && !update.is_delete() {
            fast_forward_only = true;
        }
    }

    let config: &[(&str, &str)] = if fast_forward_only {
        &[("receive.denyNonFastForwards", "true")]
    } else {
        &[]
    };
    let mut child =
        protocol::spawn_service_rpc(repo_path, "receive-pack", config).map_err(internal_error)?;
    let (Some(mut stdin), Some(mut stdout)) = (child.stdin.take(), child.stdout.take()) else {
        return Err(internal_error("git pipes unavailable"));
    };

    // Owns stdin: git sees the end of the request only when the pipe is
    // closed (dropped), e.g. after a truncated pack.
    let feed = async move {
        stdin.write_all(&prefix).await?;
        let mut limited = input.take(MAX_PUSH_BYTES + 1);
        let copied = tokio::io::copy(&mut limited, &mut stdin).await?;
        if copied > MAX_PUSH_BYTES {
            return Err(std::io::Error::from(std::io::ErrorKind::FileTooLarge));
        }
        Ok(())
    };
    let respond = tokio::io::copy(&mut stdout, output);
    tokio::pin!(feed, respond);
    // Done when git is: a client may wait for the report before it closes
    // its side (SSH), and git stops reading when it rejects a push.
    let responded = tokio::select! {
        responded = &mut respond => responded,
        fed = &mut feed => {
            if let Err(e) = fed {
                if e.kind() == std::io::ErrorKind::FileTooLarge {
                    let _ = child.kill().await;
                    return Err((StatusCode::PAYLOAD_TOO_LARGE, "push too large".into()));
                }
                // git's output says why it stopped reading.
                tracing::debug!("receive-pack request stream ended: {}", e);
            }
            respond.await
        }
    };
    responded.map_err(internal_error)?;
    match tokio::time::timeout(RPC_TIMEOUT, child.wait()).await {
        Ok(Ok(status)) if !status.success() => {
            tracing::debug!(%status, "git receive-pack exited with error");
        }
        Ok(Err(e)) => return Err(internal_error(e)),
        Err(_) => {
            let _ = child.kill().await;
            return Err(internal_error("git receive-pack timed out"));
        }
        Ok(Ok(_)) => {}
    }

    // Fire webhooks and pipelines for the updates git actually applied.
    let applied: Vec<RefUpdate> = commands
        .updates
        .into_iter()
        .filter(|u| {
            let current = delta_vcs::refs::ref_target(repo_path, &u.refname);
            if u.is_delete() {
                current.is_none()
            } else {
                current.as_deref() == Some(u.new.as_str())
            }
        })
        .collect();
    if !applied.is_empty() {
        let state = state.clone();
        let owner = owner.to_string();
        let repo = repo.clone();
        let repo_path = repo_path.to_path_buf();
        let pusher = pusher.clone();
        tokio::spawn(async move {
            dispatch_push_events(&state, &owner, &repo, &repo_path, &pusher, &applied).await;
        });
    }
    Ok(())
}

/// Look up the repository record for `owner/name`.
pub(crate) async fn find_repo(
    state: &AppState,
    owner: &str,
    name: &str,
) -> Result<Repository, HttpError> {
    let not_found = || (StatusCode::NOT_FOUND, "repository not found".to_string());
    let owner_user = delta_core::db::user::get_by_username(&state.db, owner)
        .await
        .map_err(|_| not_found())?;
    delta_core::db::repo::get_by_owner_and_name(&state.db, &owner_user.id.to_string(), name)
        .await
        .map_err(|_| not_found())
}

/// Authenticate the request and require push access to the repository:
/// 401 without valid credentials, 403 if the user may not push.
async fn authorize_push(
    state: &AppState,
    headers: &HeaderMap,
    owner: &str,
    name: &str,
) -> Result<(User, Repository), HttpError> {
    let user = authenticate_git_user(state, headers)
        .await
        .map_err(|e| (StatusCode::UNAUTHORIZED, e))?;
    let repo = find_repo(state, owner, name).await?;

    // Owner always has push access
    if repo.owner != user.id.to_string() {
        let role = delta_core::db::collaborator::get_role(
            &state.db,
            &repo.id.to_string(),
            &user.id.to_string(),
        )
        .await
        .unwrap_or(None);
        if !role.is_some_and(|r| r.has(delta_core::models::collaborator::CollaboratorRole::Write)) {
            return Err((
                StatusCode::FORBIDDEN,
                "you don't have push access to this repository".into(),
            ));
        }
    }
    Ok((user, repo))
}

/// Check read access — public repos are open, private repos need owner or collaborator auth.
async fn check_read_access(
    state: &AppState,
    owner: &str,
    name: &str,
    headers: &HeaderMap,
) -> Result<(), HttpError> {
    let repo = find_repo(state, owner, name).await?;
    if repo.visibility == Visibility::Public {
        return Ok(());
    }

    // Private repo — authenticate and check access
    let user = authenticate_git_user(state, headers)
        .await
        .map_err(|e| (StatusCode::UNAUTHORIZED, e))?;
    if repo.owner == user.id.to_string() {
        return Ok(());
    }
    let role = delta_core::db::collaborator::get_role(
        &state.db,
        &repo.id.to_string(),
        &user.id.to_string(),
    )
    .await
    .unwrap_or(None);
    if role.is_some() {
        Ok(())
    } else {
        Err((StatusCode::NOT_FOUND, "repository not found".into()))
    }
}

/// Authenticate a git HTTP request (Basic auth with an API token as the
/// password) and return the User. Does NOT check repository permissions.
async fn authenticate_git_user(
    state: &AppState,
    headers: &HeaderMap,
) -> std::result::Result<User, String> {
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

    Ok(user)
}

/// Fire webhooks and CI pipelines for the ref updates a push applied.
async fn dispatch_push_events(
    state: &AppState,
    owner: &str,
    repo: &Repository,
    repo_path: &std::path::Path,
    pusher: &User,
    updates: &[RefUpdate],
) {
    for update in updates {
        if let Err(e) = dispatch_push_webhooks(
            &state.db,
            owner,
            repo,
            &pusher.username,
            update,
            state.config.webhooks.https_only,
        )
        .await
        {
            tracing::warn!(refname = %update.refname, "webhook dispatch failed: {}", e);
        }

        if update.is_delete() {
            continue;
        }
        let target = if let Some(branch) = update.refname.strip_prefix("refs/heads/") {
            PushTarget::Branch(branch)
        } else if let Some(tag) = update.refname.strip_prefix("refs/tags/") {
            PushTarget::Tag(tag)
        } else {
            continue;
        };
        if let Err(e) = dispatch_push_pipelines(state, repo, repo_path, target, &update.new).await {
            tracing::warn!(refname = %update.refname, "pipeline dispatch failed: {}", e);
        }
    }
}

/// Dispatch push webhooks for one updated ref.
async fn dispatch_push_webhooks(
    db: &sqlx::SqlitePool,
    owner: &str,
    repo: &Repository,
    pusher: &str,
    update: &RefUpdate,
    https_only: bool,
) -> delta_core::Result<()> {
    let webhooks = delta_core::db::webhook::get_for_event(db, &repo.id.to_string(), "push").await?;
    if webhooks.is_empty() {
        return Ok(());
    }

    let payload = serde_json::json!({
        "event": "push",
        "repo_owner": owner,
        "repo_name": repo.name,
        "pusher": pusher,
        "ref": update.refname,
        "before": update.old,
        "after": update.new,
        "created": update.is_create(),
        "deleted": update.is_delete(),
        "timestamp": chrono::Utc::now().to_rfc3339(),
    });
    let payload_str = serde_json::to_string(&payload)?;
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
        // Resolve and vet the target now (not just when the webhook was
        // created) and pin the connection to the vetted addresses.
        let client =
            match crate::ssrf::guarded_client(&webhook.url, std::time::Duration::from_secs(10))
                .await
            {
                Ok(client) => client,
                Err(e) => {
                    tracing::warn!(webhook_id = %webhook.id, "skipping webhook: {}", e);
                    continue;
                }
            };

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
                let body = crate::ssrf::read_capped(r, 64 * 1024).await;
                (
                    Some(status),
                    Some(String::from_utf8_lossy(&body).into_owned()),
                )
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

/// What a push updated, for pipeline triggers.
#[derive(Clone, Copy)]
enum PushTarget<'a> {
    Branch(&'a str),
    Tag(&'a str),
}

/// Trigger CI/CD pipelines for a pushed branch or tag at `commit_sha`.
///
/// Pipelines run in a temporary checkout of the commit: the hosted bare
/// repository has no working tree, and build steps must never be able to
/// write to it.
async fn dispatch_push_pipelines(
    state: &AppState,
    repo: &Repository,
    repo_path: &std::path::Path,
    target: PushTarget<'_>,
    commit_sha: &str,
) -> std::result::Result<(), String> {
    let db = &state.db;
    let repo_id = repo.id.to_string();
    let ci_config = &state.config.ci;

    let checkout = delta_vcs::checkout::checkout_commit(repo_path, commit_sha)
        .await
        .map_err(|e| format!("failed to check out {commit_sha}: {e}"))?;
    let work_dir = checkout.work_dir();
    let home_dir = checkout.home_dir();

    // Decrypt repo secrets for pipeline env
    let encryption_key = delta_core::crypto::derive_key(&state.config.auth.secrets_key);
    let mut secrets = std::collections::HashMap::new();
    if let Ok(encrypted_secrets) = delta_core::db::secret::get_all_values(db, &repo_id).await {
        for (key, encrypted_value) in encrypted_secrets {
            match delta_core::crypto::decrypt(&encryption_key, &encrypted_value) {
                Ok(value) => {
                    secrets.insert(key, value);
                }
                Err(e) => tracing::error!(repo_id, secret = key, "failed to decrypt secret: {}", e),
            }
        }
    }

    let ctx = delta_ci::runner::PipelineContext {
        pool: db,
        repo_id: &repo_id,
        repo_path: &work_dir,
        home_dir: Some(&home_dir),
        commit_sha,
        secrets: &secrets,
        streams: Some(&state.pipeline_streams),
        sandbox: resolve_sandbox_mode(ci_config),
        runners_enabled: ci_config.runner_token.is_some(),
    };
    match target {
        PushTarget::Branch(branch) => delta_ci::runner::run_push_pipelines(&ctx, branch).await,
        PushTarget::Tag(tag) => delta_ci::runner::run_tag_pipelines(&ctx, tag).await,
    }
    Ok(())
}

/// Determine the sandbox mode based on CI config and system capabilities.
fn resolve_sandbox_mode(
    ci_config: &delta_core::config::CiConfig,
) -> delta_ci::executor::SandboxMode {
    use delta_ci::executor::SandboxMode;

    if !ci_config.sandbox_enabled {
        return SandboxMode::None;
    }

    // Try Landlock first (Linux only)
    #[cfg(target_os = "linux")]
    {
        if delta_ci::sandbox::landlock_supported() {
            tracing::info!("CI sandbox: using Landlock + seccomp");
            return SandboxMode::Landlock;
        }
        tracing::warn!("CI sandbox: Landlock not supported by kernel");
    }

    // Fall back to container runtime
    match &ci_config.container_runtime {
        delta_core::config::ContainerRuntime::None => {
            tracing::warn!("CI sandbox: no sandboxing available");
            SandboxMode::None
        }
        delta_core::config::ContainerRuntime::Auto => {
            if let Some(runtime) = delta_ci::container::detect_runtime() {
                tracing::info!("CI sandbox: using container runtime '{}'", runtime);
                SandboxMode::Container {
                    runtime,
                    image: "alpine:latest".to_string(),
                }
            } else {
                tracing::warn!("CI sandbox: no container runtime found, running unsandboxed");
                SandboxMode::None
            }
        }
        delta_core::config::ContainerRuntime::Podman => SandboxMode::Container {
            runtime: "podman".to_string(),
            image: "alpine:latest".to_string(),
        },
        delta_core::config::ContainerRuntime::Docker => SandboxMode::Container {
            runtime: "docker".to_string(),
            image: "alpine:latest".to_string(),
        },
    }
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
}
