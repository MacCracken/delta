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

use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/{owner}/{repo}/info/lfs/objects/batch", post(batch))
        .route("/{owner}/{repo}/info/lfs/objects/verify", post(verify))
        .route(
            "/{owner}/{repo}/info/lfs/objects/{oid}",
            get(download).put(upload),
        )
}

/// Strip `.git` suffix from repo segment.
fn parse_repo_name(repo: &str) -> &str {
    repo.strip_suffix(".git").unwrap_or(repo)
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
    let (repo_record, _is_owner) = if req.operation == "upload" {
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

    // Build base URL for object actions
    let base_url = format!("/{}/{}/info/lfs/objects", owner, repo);

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
                                href: format!("/{}/{}/info/lfs/objects/verify", owner, repo),
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

    let (repo_record, _) = resolve_repo_and_auth(&state, &headers, &owner, name).await?;
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

/// PUT /{owner}/{repo}.git/info/lfs/objects/{oid}
async fn upload(
    State(state): State<AppState>,
    Path((owner, repo, oid)): Path<(String, String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, (StatusCode, String)> {
    let name = parse_repo_name(&repo);

    let (repo_record, _) = resolve_repo_and_auth_write(&state, &headers, &owner, name).await?;
    let repo_id = repo_record.id.to_string();

    if !delta_registry::lfs_store::validate_oid(&oid) {
        return Err((StatusCode::BAD_REQUEST, "invalid OID".into()));
    }

    // Store with SHA-256 verification
    state
        .lfs_store
        .store_verified(&body, &oid)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    // Record in DB
    let size = body.len() as i64;
    let _ = db::lfs::create(&state.db, &repo_id, &oid, size)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

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

    let (repo_record, _) = resolve_repo_and_auth(&state, &headers, &owner, name).await?;
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
/// Returns (repo, is_owner).
async fn resolve_repo_and_auth(
    state: &AppState,
    headers: &HeaderMap,
    owner: &str,
    name: &str,
) -> Result<(delta_core::models::repo::Repository, bool), (StatusCode, String)> {
    let owner_user = db::user::get_by_username(&state.db, owner)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "repository not found".into()))?;

    let owner_id = owner_user.id.to_string();
    let repo = db::repo::get_by_owner_and_name(&state.db, &owner_id, name)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "repository not found".into()))?;

    // Public repos allow anonymous reads
    if repo.visibility == delta_core::models::repo::Visibility::Public {
        return Ok((repo, false));
    }

    // Private repo — need auth
    let user = authenticate_lfs_user(state, headers)
        .await
        .map_err(|e| (StatusCode::UNAUTHORIZED, e))?;

    let is_owner = user.username == owner;
    if !is_owner {
        let role =
            db::collaborator::get_role(&state.db, &repo.id.to_string(), &user.id.to_string())
                .await
                .unwrap_or(None);
        if role.is_none() {
            return Err((StatusCode::NOT_FOUND, "repository not found".into()));
        }
    }

    Ok((repo, is_owner))
}

/// Resolve repo and authenticate for write access.
async fn resolve_repo_and_auth_write(
    state: &AppState,
    headers: &HeaderMap,
    owner: &str,
    name: &str,
) -> Result<(delta_core::models::repo::Repository, bool), (StatusCode, String)> {
    let owner_user = db::user::get_by_username(&state.db, owner)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "repository not found".into()))?;

    let owner_id = owner_user.id.to_string();
    let repo = db::repo::get_by_owner_and_name(&state.db, &owner_id, name)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "repository not found".into()))?;

    let user = authenticate_lfs_user(state, headers)
        .await
        .map_err(|e| (StatusCode::UNAUTHORIZED, e))?;

    let is_owner = user.username == owner;
    if !is_owner {
        let role =
            db::collaborator::get_role(&state.db, &repo.id.to_string(), &user.id.to_string())
                .await
                .unwrap_or(None);
        match role {
            Some(r) if r.has(delta_core::models::collaborator::CollaboratorRole::Write) => {
                // allowed
            }
            _ => {
                return Err((
                    StatusCode::FORBIDDEN,
                    "no write access to this repository".into(),
                ));
            }
        }
    }

    Ok((repo, is_owner))
}

/// Authenticate an LFS request. LFS uses Basic auth (same as git HTTP).
fn authenticate_lfs_user(
    state: &AppState,
    headers: &HeaderMap,
) -> impl std::future::Future<Output = std::result::Result<delta_core::models::user::User, String>> + Send
{
    let auth_header = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    let db = state.db.clone();

    async move {
        let auth = auth_header.ok_or("authentication required")?;

        // Support both Basic and Bearer auth
        if let Some(token) = auth.strip_prefix("Bearer ") {
            return crate::auth::authenticate_token(&db, token)
                .await
                .map_err(|_| "invalid or expired token".to_string());
        }

        let credentials = auth.strip_prefix("Basic ").ok_or("invalid auth format")?;

        let decoded =
            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, credentials)
                .map_err(|_| "invalid base64 credentials".to_string())?;

        let decoded_str =
            String::from_utf8(decoded).map_err(|_| "invalid utf-8 credentials".to_string())?;

        let (username, token) = decoded_str
            .split_once(':')
            .ok_or("invalid credential format")?;

        let user = crate::auth::authenticate_token(&db, token)
            .await
            .map_err(|_| "invalid or expired token".to_string())?;

        if user.username != username {
            return Err("username mismatch".to_string());
        }

        Ok(user)
    }
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
