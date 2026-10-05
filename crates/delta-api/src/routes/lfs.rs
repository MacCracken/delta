//! Git LFS Batch API routes.
//!
//! Implements the Git LFS Batch API specification:
//!   POST /{owner}/{repo}.git/info/lfs/objects/batch
//!   GET  /{owner}/{repo}.git/info/lfs/objects/{oid}
//!   PUT  /{owner}/{repo}.git/info/lfs/objects/{oid}
//!   POST /{owner}/{repo}.git/info/lfs/objects/verify

use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use delta_core::db;
use serde::{Deserialize, Serialize};

use crate::routes::git::{GitAccess, authorize_git_access};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/{owner}/{repo}/info/lfs/objects/batch", post(batch))
        .route("/{owner}/{repo}/info/lfs/objects/verify", post(verify))
        .route(
            "/{owner}/{repo}/info/lfs/objects/{oid}",
            get(download).put(upload),
        )
        .layer(axum::extract::DefaultBodyLimit::max(
            crate::helpers::MAX_UPLOAD_BYTES,
        ))
        // git-lfs asks the credential helper only after a challenged 401.
        .layer(axum::middleware::map_response(
            crate::routes::git::add_basic_auth_challenge,
        ))
}

/// Strip `.git` suffix from repo segment.
fn parse_repo_name(repo: &str) -> &str {
    crate::helpers::strip_git_suffix(repo)
}

// --- LFS Batch API types ---

#[derive(Deserialize)]
struct BatchRequest {
    operation: String,
    objects: Vec<BatchObject>,
    #[serde(default)]
    transfers: Vec<String>,
}

#[derive(Deserialize, Serialize, Clone)]
struct BatchObject {
    oid: String,
    size: i64,
}

#[derive(Serialize)]
struct BatchResponse {
    transfer: String,
    objects: Vec<BatchObjectResponse>,
}

#[derive(Serialize)]
struct BatchObjectResponse {
    oid: String,
    size: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    actions: Option<BatchActions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<BatchError>,
}

#[derive(Serialize)]
struct BatchActions {
    #[serde(skip_serializing_if = "Option::is_none")]
    download: Option<BatchAction>,
    #[serde(skip_serializing_if = "Option::is_none")]
    upload: Option<BatchAction>,
    #[serde(skip_serializing_if = "Option::is_none")]
    verify: Option<BatchAction>,
}

#[derive(Serialize)]
struct BatchAction {
    href: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    header: Option<std::collections::HashMap<String, String>>,
    expires_in: i64,
}

#[derive(Serialize)]
struct BatchError {
    code: u16,
    message: String,
}

/// Scheme and authority clients use to reach this server, without a
/// trailing slash: `server.external_url`, else `federation.instance_url`,
/// else derived from the request's Host header.
fn external_base_url(state: &AppState, headers: &HeaderMap) -> String {
    if let Some(url) = state.config.server.external_url.as_deref().or(state
        .config
        .federation
        .instance_url
        .as_deref())
    {
        return url.trim_end_matches('/').to_string();
    }
    let host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .filter(|h| !h.is_empty() && !h.contains(['/', ' ', '@']))
        .unwrap_or("localhost");
    let scheme = if state.config.server.trust_forwarded_for {
        headers
            .get("x-forwarded-proto")
            .and_then(|v| v.to_str().ok())
            .filter(|p| *p == "https" || *p == "http")
            .unwrap_or("http")
    } else {
        "http"
    };
    format!("{scheme}://{host}")
}

/// POST /{owner}/{repo}.git/info/lfs/objects/batch
///
/// The main LFS batch API endpoint. Clients send a list of objects they
/// want to download or upload, and we return action URLs for each.
async fn batch(
    State(state): State<AppState>,
    Path((owner, repo)): Path<(String, String)>,
    headers: HeaderMap,
    Json(req): Json<BatchRequest>,
) -> Result<Response, (StatusCode, String)> {
    let name = parse_repo_name(&repo);

    // Validate operation
    if req.operation != "download" && req.operation != "upload" {
        return Err((StatusCode::BAD_REQUEST, "invalid operation".into()));
    }

    // Resolve repo — uploads require write access, downloads require read
    let repo_record = if req.operation == "upload" {
        resolve_repo_and_auth_write(&state, &headers, &owner, name).await?
    } else {
        resolve_repo_and_auth(&state, &headers, &owner, name).await?
    };
    let repo_id = repo_record.id.to_string();

    // We only support "basic" transfer
    let _transfer = if req.transfers.is_empty() || req.transfers.contains(&"basic".to_string()) {
        "basic"
    } else {
        return Err((
            StatusCode::BAD_REQUEST,
            "only basic transfer adapter is supported".into(),
        ));
    };

    // Build base URL for object actions. git-lfs needs absolute URLs.
    let origin = external_base_url(&state, &headers);
    let base_url = format!("{origin}/{owner}/{repo}/info/lfs/objects");

    // Forward auth header for action URLs
    let auth_header = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    let mut objects = Vec::new();

    for obj in &req.objects {
        // Validate OID format
        if !delta_registry::lfs_store::validate_oid(&obj.oid) {
            objects.push(BatchObjectResponse {
                oid: obj.oid.clone(),
                size: obj.size,
                actions: None,
                error: Some(BatchError {
                    code: 422,
                    message: "invalid OID format".into(),
                }),
            });
            continue;
        }

        if obj.size < 0 {
            objects.push(BatchObjectResponse {
                oid: obj.oid.clone(),
                size: obj.size,
                actions: None,
                error: Some(BatchError {
                    code: 422,
                    message: "invalid size".into(),
                }),
            });
            continue;
        }

        let exists_in_db = db::lfs::exists(&state.db, &repo_id, &obj.oid)
            .await
            .unwrap_or(false);
        let exists_on_disk = state.lfs_store.exists(&obj.oid);

        let mut action_headers = std::collections::HashMap::new();
        if let Some(ref auth) = auth_header {
            action_headers.insert("Authorization".to_string(), auth.clone());
        }
        let header_map = if action_headers.is_empty() {
            None
        } else {
            Some(action_headers)
        };

        match req.operation.as_str() {
            "download" => {
                if exists_in_db && exists_on_disk {
                    objects.push(BatchObjectResponse {
                        oid: obj.oid.clone(),
                        size: obj.size,
                        actions: Some(BatchActions {
                            download: Some(BatchAction {
                                href: format!("{}/{}", base_url, obj.oid),
                                header: header_map,
                                expires_in: 3600,
                            }),
                            upload: None,
                            verify: None,
                        }),
                        error: None,
                    });
                } else {
                    objects.push(BatchObjectResponse {
                        oid: obj.oid.clone(),
                        size: obj.size,
                        actions: None,
                        error: Some(BatchError {
                            code: 404,
                            message: "object not found".into(),
                        }),
                    });
                }
            }
            "upload" => {
                if exists_in_db && exists_on_disk {
                    // Already have it — no actions needed
                    objects.push(BatchObjectResponse {
                        oid: obj.oid.clone(),
                        size: obj.size,
                        actions: None,
                        error: None,
                    });
                } else {
                    objects.push(BatchObjectResponse {
                        oid: obj.oid.clone(),
                        size: obj.size,
                        actions: Some(BatchActions {
                            download: None,
                            upload: Some(BatchAction {
                                href: format!("{}/{}", base_url, obj.oid),
                                header: header_map.clone(),
                                expires_in: 3600,
                            }),
                            verify: Some(BatchAction {
                                href: format!("{base_url}/verify"),
                                header: header_map,
                                expires_in: 3600,
                            }),
                        }),
                        error: None,
                    });
                }
            }
            _ => unreachable!(),
        }
    }

    let resp = BatchResponse {
        transfer: "basic".into(),
        objects,
    };

    Ok((
        StatusCode::OK,
        [(
            header::CONTENT_TYPE,
            "application/vnd.git-lfs+json".to_string(),
        )],
        serde_json::to_string(&resp).unwrap(),
    )
        .into_response())
}

/// GET /{owner}/{repo}.git/info/lfs/objects/{oid}
async fn download(
    State(state): State<AppState>,
    Path((owner, repo, oid)): Path<(String, String, String)>,
    headers: HeaderMap,
) -> Result<Response, (StatusCode, String)> {
    let name = parse_repo_name(&repo);

    let repo_record = resolve_repo_and_auth(&state, &headers, &owner, name).await?;
    let repo_id = repo_record.id.to_string();

    if !delta_registry::lfs_store::validate_oid(&oid) {
        return Err((StatusCode::BAD_REQUEST, "invalid OID".into()));
    }

    let exists = db::lfs::exists(&state.db, &repo_id, &oid)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if !exists {
        return Err((StatusCode::NOT_FOUND, "object not found".into()));
    }

    let data = state
        .lfs_store
        .read(&oid)
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;

    Ok((
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/octet-stream".to_string())],
        data,
    )
        .into_response())
}

/// Maximum upload body size: 100 MB.
const MAX_UPLOAD_BODY_SIZE: usize = 100 * 1024 * 1024;

/// PUT /{owner}/{repo}.git/info/lfs/objects/{oid}
async fn upload(
    State(state): State<AppState>,
    Path((owner, repo, oid)): Path<(String, String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, (StatusCode, String)> {
    if body.len() > MAX_UPLOAD_BODY_SIZE {
        return Err((
            StatusCode::PAYLOAD_TOO_LARGE,
            "request body exceeds 100 MB limit".into(),
        ));
    }

    let name = parse_repo_name(&repo);

    let repo_record = resolve_repo_and_auth_write(&state, &headers, &owner, name).await?;
    let repo_id = repo_record.id.to_string();

    if !delta_registry::lfs_store::validate_oid(&oid) {
        return Err((StatusCode::BAD_REQUEST, "invalid OID".into()));
    }

    // Store with SHA-256 verification
    state
        .lfs_store
        .store_verified(&body, &oid)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    // Record in DB. A retried or concurrent upload of the same object finds
    // it already recorded, which is success.
    let size = body.len() as i64;
    match db::lfs::create(&state.db, &repo_id, &oid, size).await {
        Ok(_) | Err(delta_core::DeltaError::Conflict(_)) => {}
        Err(e) => {
            tracing::error!("failed to record LFS object: {}", e);
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal server error".into(),
            ));
        }
    }

    Ok(StatusCode::OK.into_response())
}

/// POST /{owner}/{repo}.git/info/lfs/objects/verify
async fn verify(
    State(state): State<AppState>,
    Path((owner, repo)): Path<(String, String)>,
    headers: HeaderMap,
    Json(req): Json<BatchObject>,
) -> Result<Response, (StatusCode, String)> {
    let name = parse_repo_name(&repo);

    let repo_record = resolve_repo_and_auth(&state, &headers, &owner, name).await?;
    let repo_id = repo_record.id.to_string();

    if !delta_registry::lfs_store::validate_oid(&req.oid) {
        return Err((StatusCode::BAD_REQUEST, "invalid OID".into()));
    }

    let exists = db::lfs::exists(&state.db, &repo_id, &req.oid)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if !exists || !state.lfs_store.exists(&req.oid) {
        return Err((StatusCode::NOT_FOUND, "object not found".into()));
    }

    // Verify size matches
    let disk_size = state
        .lfs_store
        .size(&req.oid)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if disk_size as i64 != req.size {
        return Err((StatusCode::BAD_REQUEST, "size mismatch".into()));
    }

    Ok(StatusCode::OK.into_response())
}

/// Resolve the repo from owner/name and authenticate for read access.
async fn resolve_repo_and_auth(
    state: &AppState,
    headers: &HeaderMap,
    owner: &str,
    name: &str,
) -> Result<delta_core::models::repo::Repository, (StatusCode, String)> {
    let (_, repo) = authorize_git_access(state, headers, owner, name, GitAccess::Read).await?;
    Ok(repo)
}

/// Resolve repo and authenticate for write access.
async fn resolve_repo_and_auth_write(
    state: &AppState,
    headers: &HeaderMap,
    owner: &str,
    name: &str,
) -> Result<delta_core::models::repo::Repository, (StatusCode, String)> {
    let (_, repo) = authorize_git_access(state, headers, owner, name, GitAccess::Write).await?;
    Ok(repo)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_repo_name() {
        assert_eq!(parse_repo_name("myrepo.git"), "myrepo");
        assert_eq!(parse_repo_name("myrepo"), "myrepo");
    }
}
