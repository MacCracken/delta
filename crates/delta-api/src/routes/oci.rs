//! OCI Distribution Spec routes for container image registry.

use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    routing::{get, head, patch, post},
};
use delta_core::db;
use delta_registry::oci::{OciStagingArea, sha256_digest};
use serde::{Deserialize, Serialize};

use delta_core::models::collaborator::CollaboratorRole;

use crate::helpers::{require_role, resolve_repo_authed};
use crate::state::AppState;

/// Maximum upload body size: 100 MB.
const MAX_UPLOAD_BODY_SIZE: usize = 100 * 1024 * 1024;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/v2/", get(version_check))
        .route(
            "/v2/{owner}/{name}/blobs/{digest}",
            head(check_blob).get(pull_blob).delete(delete_blob),
        )
        .route("/v2/{owner}/{name}/blobs/uploads/", post(initiate_upload))
        .route(
            "/v2/{owner}/{name}/blobs/uploads/{uuid}",
            patch(upload_chunk).put(complete_upload),
        )
        .route(
            "/v2/{owner}/{name}/manifests/{reference}",
            head(check_manifest)
                .get(pull_manifest)
                .put(push_manifest)
                .delete(oci_delete_manifest),
        )
        .route("/v2/{owner}/{name}/tags/list", get(list_tags))
        .layer(axum::extract::DefaultBodyLimit::max(
            crate::helpers::MAX_UPLOAD_BYTES,
        ))
        .layer(axum::middleware::map_response(oci_auth_challenge))
}

/// Registry clients (docker, podman, skopeo) only send credentials after a
/// 401 carrying a challenge, and expect errors in the distribution-spec
/// JSON format.
async fn oci_auth_challenge(response: axum::response::Response) -> axum::response::Response {
    use axum::http::{HeaderValue, header};
    if response.status() != StatusCode::UNAUTHORIZED {
        return response;
    }
    let (mut parts, _) = response.into_parts();
    parts.headers.insert(
        header::WWW_AUTHENTICATE,
        HeaderValue::from_static("Basic realm=\"Delta\""),
    );
    parts.headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    parts.headers.remove(header::CONTENT_LENGTH);
    let body = r#"{"errors":[{"code":"UNAUTHORIZED","message":"authentication required"}]}"#;
    axum::response::Response::from_parts(parts, axum::body::Body::from(body))
}

/// Caller identity on OCI routes: a Bearer token, or HTTP Basic
/// `username:token` as sent by `docker login`. `None` without credentials.
///
/// Basic auth is accepted only here (and on git routes): browsers replay
/// cached Basic credentials automatically, so accepting it API-wide would
/// expose the rest of the API to cross-site requests.
struct OciUser(Option<delta_core::models::user::User>);

impl axum::extract::FromRequestParts<AppState> for OciUser {
    type Rejection = (StatusCode, String);

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let Some(header) = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
        else {
            return Ok(OciUser(None));
        };
        let unauthorized = || (StatusCode::UNAUTHORIZED, "invalid credentials".to_string());
        let user = if let Some(token) = header.strip_prefix("Bearer ") {
            crate::auth::authenticate_token(&state.db, token)
                .await
                .map_err(|_| unauthorized())?
        } else if let Some(encoded) = header.strip_prefix("Basic ") {
            let decoded =
                base64::Engine::decode(&base64::engine::general_purpose::STANDARD, encoded)
                    .map_err(|_| unauthorized())?;
            let decoded = String::from_utf8(decoded).map_err(|_| unauthorized())?;
            let (username, token) = decoded.split_once(':').ok_or_else(unauthorized)?;
            let user = crate::auth::authenticate_token(&state.db, token)
                .await
                .map_err(|_| unauthorized())?;
            if user.username != username {
                return Err(unauthorized());
            }
            user
        } else {
            return Err(unauthorized());
        };
        Ok(OciUser(Some(user)))
    }
}

/// Require credentials (writes always do).
fn require_user(
    user: Option<delta_core::models::user::User>,
) -> Result<delta_core::models::user::User, (StatusCode, String)> {
    user.ok_or((StatusCode::UNAUTHORIZED, "authentication required".into()))
}

/// Resolve a repository for reading: public repositories are readable
/// anonymously; anything else asks for credentials.
async fn resolve_readable(
    state: &AppState,
    owner: &str,
    name: &str,
    user: Option<&delta_core::models::user::User>,
) -> Result<
    (
        delta_core::models::repo::Repository,
        delta_core::models::user::User,
    ),
    (StatusCode, String),
> {
    match user {
        Some(user) => resolve_repo_authed(state, owner, name, user).await,
        None => crate::helpers::resolve_repo(state, owner, name)
            .await
            .map_err(|_| (StatusCode::UNAUTHORIZED, "authentication required".into())),
    }
}

// --- Version Check ---

/// `GET /v2/` — clients probe this to discover the auth scheme, so it
/// requires credentials (`docker login` checks them here).
async fn version_check(
    OciUser(user): OciUser,
) -> Result<
    (
        StatusCode,
        [(&'static str, &'static str); 1],
        Json<serde_json::Value>,
    ),
    (StatusCode, String),
> {
    require_user(user)?;
    Ok((
        StatusCode::OK,
        [("Docker-Distribution-API-Version", "registry/2.0")],
        Json(serde_json::json!({})),
    ))
}

// --- Blobs ---

async fn check_blob(
    State(state): State<AppState>,
    Path((owner, name, digest)): Path<(String, String, String)>,
    OciUser(user): OciUser,
) -> Result<(StatusCode, HeaderMap), (StatusCode, String)> {
    let (repo, _) = resolve_readable(&state, &owner, &name, user.as_ref()).await?;
    let blob = db::oci::get_repo_blob(&state.db, &repo.id.to_string(), &digest)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "blob not found".into()))?;

    let mut headers = HeaderMap::new();
    headers.insert("docker-content-digest", digest.parse().unwrap());
    headers.insert(
        header::CONTENT_LENGTH,
        blob.size_bytes.to_string().parse().unwrap(),
    );
    Ok((StatusCode::OK, headers))
}

async fn pull_blob(
    State(state): State<AppState>,
    Path((owner, name, digest)): Path<(String, String, String)>,
    OciUser(user): OciUser,
) -> Result<(StatusCode, HeaderMap, Vec<u8>), (StatusCode, String)> {
    let (repo, _) = resolve_readable(&state, &owner, &name, user.as_ref()).await?;
    let blob = db::oci::get_repo_blob(&state.db, &repo.id.to_string(), &digest)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "blob not found".into()))?;

    let data = state
        .blob_store
        .read(&blob.content_hash)
        .map_err(|e| (StatusCode::NOT_FOUND, format!("blob data not found: {}", e)))?;

    let mut headers = HeaderMap::new();
    headers.insert("docker-content-digest", digest.parse().unwrap());
    headers.insert(
        header::CONTENT_LENGTH,
        data.len().to_string().parse().unwrap(),
    );
    headers.insert(
        header::CONTENT_TYPE,
        "application/octet-stream".parse().unwrap(),
    );

    Ok((StatusCode::OK, headers, data))
}

async fn delete_blob(
    State(state): State<AppState>,
    Path((owner, name, digest)): Path<(String, String, String)>,
    OciUser(user): OciUser,
) -> Result<StatusCode, (StatusCode, String)> {
    let user = require_user(user)?;
    let (repo, owner_user) = resolve_repo_authed(&state, &owner, &name, &user).await?;
    require_role(&state, &repo, &owner_user, &user, CollaboratorRole::Write).await?;

    let blob = db::oci::get_repo_blob(&state.db, &repo.id.to_string(), &digest)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "blob not found".into()))?;

    db::oci::delete_repo_blob(&state.db, &repo.id.to_string(), &digest)
        .await
        .map_err(|e| {
            tracing::error!("failed to delete blob: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal server error".into(),
            )
        })?;

    if let Err(e) =
        delta_registry::store::release_blob(&state.db, &state.blob_store, &blob.content_hash).await
    {
        tracing::warn!("failed to release OCI blob: {}", e);
    }
    Ok(StatusCode::ACCEPTED)
}

// --- Uploads ---

#[derive(Deserialize)]
struct UploadQuery {
    digest: Option<String>,
}

async fn initiate_upload(
    State(state): State<AppState>,
    Path((owner, name)): Path<(String, String)>,
    OciUser(user): OciUser,
    Query(query): Query<UploadQuery>,
    body: Bytes,
) -> Result<(StatusCode, HeaderMap), (StatusCode, String)> {
    if body.len() > MAX_UPLOAD_BODY_SIZE {
        return Err((
            StatusCode::PAYLOAD_TOO_LARGE,
            "request body exceeds 100 MB limit".into(),
        ));
    }

    let user = require_user(user)?;
    let (repo, owner_user) = resolve_repo_authed(&state, &owner, &name, &user).await?;
    require_role(&state, &repo, &owner_user, &user, CollaboratorRole::Write).await?;

    let repo_id = repo.id.to_string();

    // Monolithic upload if digest is provided
    if let Some(digest) = query.digest {
        let staging = OciStagingArea::new(&state.config.storage.artifacts_dir);
        let (content_hash, size) = staging
            .store_monolithic(&body, &digest, &state.blob_store)
            .map_err(|e| (StatusCode::BAD_REQUEST, e))?;

        db::oci::upsert_repo_blob(&state.db, &repo_id, &digest, &content_hash, size)
            .await
            .map_err(|e| {
                tracing::error!("failed to store blob record: {}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal server error".into(),
                )
            })?;

        let mut headers = HeaderMap::new();
        headers.insert(
            header::LOCATION,
            format!("/v2/{}/{}/blobs/{}", owner, name, digest)
                .parse()
                .unwrap(),
        );
        headers.insert("docker-content-digest", digest.parse().unwrap());
        return Ok((StatusCode::CREATED, headers));
    }

    // Chunked upload: create session
    let upload_id = db::oci::create_blob_upload(&state.db, &repo_id)
        .await
        .map_err(|e| {
            tracing::error!("failed to create upload: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal server error".into(),
            )
        })?;

    let mut headers = HeaderMap::new();
    headers.insert(
        header::LOCATION,
        format!("/v2/{}/{}/blobs/uploads/{}", owner, name, upload_id)
            .parse()
            .unwrap(),
    );
    headers.insert("docker-upload-uuid", upload_id.parse().unwrap());
    headers.insert(header::RANGE, "0-0".parse().unwrap());

    Ok((StatusCode::ACCEPTED, headers))
}

async fn upload_chunk(
    State(state): State<AppState>,
    Path((owner, name, uuid)): Path<(String, String, String)>,
    OciUser(user): OciUser,
    body: Bytes,
) -> Result<(StatusCode, HeaderMap), (StatusCode, String)> {
    if body.len() > MAX_UPLOAD_BODY_SIZE {
        return Err((
            StatusCode::PAYLOAD_TOO_LARGE,
            "request body exceeds 100 MB limit".into(),
        ));
    }

    let user = require_user(user)?;
    let (repo, owner_user) = resolve_repo_authed(&state, &owner, &name, &user).await?;
    require_role(&state, &repo, &owner_user, &user, CollaboratorRole::Write).await?;

    let upload = db::oci::get_blob_upload(&state.db, &uuid)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "upload not found".into()))?;

    // Verify upload belongs to this repository
    if upload.repo_id != repo.id.to_string() {
        return Err((StatusCode::NOT_FOUND, "upload not found".into()));
    }

    if upload.state != "uploading" {
        return Err((StatusCode::BAD_REQUEST, "upload already completed".into()));
    }

    let staging = OciStagingArea::new(&state.config.storage.artifacts_dir);
    let new_offset = staging.append_chunk(&uuid, &body).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("chunk write failed: {}", e),
        )
    })?;

    db::oci::update_blob_upload_offset(&state.db, &uuid, new_offset as i64)
        .await
        .map_err(|e| {
            tracing::error!("failed to update upload offset: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal server error".into(),
            )
        })?;

    let mut headers = HeaderMap::new();
    headers.insert(
        header::LOCATION,
        format!("/v2/{}/{}/blobs/uploads/{}", owner, name, uuid)
            .parse()
            .unwrap(),
    );
    headers.insert("docker-upload-uuid", uuid.parse().unwrap());
    headers.insert(
        header::RANGE,
        format!("0-{}", new_offset.saturating_sub(1))
            .parse()
            .unwrap(),
    );

    Ok((StatusCode::ACCEPTED, headers))
}

async fn complete_upload(
    State(state): State<AppState>,
    Path((owner, name, uuid)): Path<(String, String, String)>,
    OciUser(user): OciUser,
    Query(query): Query<UploadQuery>,
    body: Bytes,
) -> Result<(StatusCode, HeaderMap), (StatusCode, String)> {
    if body.len() > MAX_UPLOAD_BODY_SIZE {
        return Err((
            StatusCode::PAYLOAD_TOO_LARGE,
            "request body exceeds 100 MB limit".into(),
        ));
    }

    let user = require_user(user)?;
    let (repo, owner_user) = resolve_repo_authed(&state, &owner, &name, &user).await?;
    require_role(&state, &repo, &owner_user, &user, CollaboratorRole::Write).await?;

    let digest = query.digest.ok_or((
        StatusCode::BAD_REQUEST,
        "digest query parameter required".into(),
    ))?;

    let upload = db::oci::get_blob_upload(&state.db, &uuid)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "upload not found".into()))?;

    // Verify upload belongs to this repository
    if upload.repo_id != repo.id.to_string() {
        return Err((StatusCode::NOT_FOUND, "upload not found".into()));
    }

    if upload.state != "uploading" {
        return Err((StatusCode::BAD_REQUEST, "upload already completed".into()));
    }

    let staging = OciStagingArea::new(&state.config.storage.artifacts_dir);

    // Append any final chunk data
    if !body.is_empty() {
        staging.append_chunk(&uuid, &body).map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("chunk write failed: {}", e),
            )
        })?;
    }

    // Finalize: verify digest and store
    let (content_hash, size) = staging
        .finalize(&uuid, &digest, &state.blob_store)
        .map_err(|e| (StatusCode::BAD_REQUEST, e))?;

    let repo_id = repo.id.to_string();

    db::oci::upsert_repo_blob(&state.db, &repo_id, &digest, &content_hash, size)
        .await
        .map_err(|e| {
            tracing::error!("failed to store blob record: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal server error".into(),
            )
        })?;

    db::oci::complete_blob_upload(&state.db, &uuid)
        .await
        .map_err(|e| {
            tracing::error!("failed to complete upload: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal server error".into(),
            )
        })?;

    let mut headers = HeaderMap::new();
    headers.insert(
        header::LOCATION,
        format!("/v2/{}/{}/blobs/{}", owner, name, digest)
            .parse()
            .unwrap(),
    );
    headers.insert("docker-content-digest", digest.parse().unwrap());

    Ok((StatusCode::CREATED, headers))
}

// --- Manifests ---

async fn check_manifest(
    State(state): State<AppState>,
    Path((owner, name, reference)): Path<(String, String, String)>,
    OciUser(user): OciUser,
) -> Result<(StatusCode, HeaderMap), (StatusCode, String)> {
    let (repo, _) = resolve_readable(&state, &owner, &name, user.as_ref()).await?;
    let manifest = resolve_manifest(&state, &repo.id.to_string(), &reference).await?;

    let mut headers = HeaderMap::new();
    headers.insert("docker-content-digest", manifest.digest.parse().unwrap());
    headers.insert(header::CONTENT_TYPE, manifest.media_type.parse().unwrap());
    headers.insert(
        header::CONTENT_LENGTH,
        manifest.size_bytes.to_string().parse().unwrap(),
    );

    Ok((StatusCode::OK, headers))
}

async fn pull_manifest(
    State(state): State<AppState>,
    Path((owner, name, reference)): Path<(String, String, String)>,
    OciUser(user): OciUser,
) -> Result<(StatusCode, HeaderMap, Vec<u8>), (StatusCode, String)> {
    let (repo, _) = resolve_readable(&state, &owner, &name, user.as_ref()).await?;
    let manifest = resolve_manifest(&state, &repo.id.to_string(), &reference).await?;

    let data = state.blob_store.read(&manifest.content_hash).map_err(|e| {
        (
            StatusCode::NOT_FOUND,
            format!("manifest data not found: {}", e),
        )
    })?;

    let mut headers = HeaderMap::new();
    headers.insert("docker-content-digest", manifest.digest.parse().unwrap());
    headers.insert(header::CONTENT_TYPE, manifest.media_type.parse().unwrap());
    headers.insert(
        header::CONTENT_LENGTH,
        data.len().to_string().parse().unwrap(),
    );

    Ok((StatusCode::OK, headers, data))
}

async fn push_manifest(
    State(state): State<AppState>,
    Path((owner, name, reference)): Path<(String, String, String)>,
    OciUser(user): OciUser,
    headers: HeaderMap,
    body: Bytes,
) -> Result<(StatusCode, HeaderMap), (StatusCode, String)> {
    let user = require_user(user)?;
    let (repo, owner_user) = resolve_repo_authed(&state, &owner, &name, &user).await?;
    require_role(&state, &repo, &owner_user, &user, CollaboratorRole::Write).await?;

    let media_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/vnd.oci.image.manifest.v1+json")
        .to_string();

    let digest = sha256_digest(&body);

    // Validate the reference before storing anything: a digest reference
    // must match the content, and a tag must be well-formed.
    let tag = if reference.starts_with("sha256:") {
        if reference != digest {
            return Err((
                StatusCode::BAD_REQUEST,
                "manifest digest does not match the reference (DIGEST_INVALID)".into(),
            ));
        }
        None
    } else {
        // Tag: 1-128 chars, alphanumeric/hyphens/dots/underscores
        if reference.is_empty()
            || reference.len() > 128
            || !reference
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.' || c == '_')
        {
            return Err((
                StatusCode::BAD_REQUEST,
                "tag must be 1-128 characters: alphanumeric, hyphens, dots, or underscores".into(),
            ));
        }
        Some(reference.as_str())
    };

    // Store manifest in blob store
    let content_hash = state.blob_store.store(&body).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("storage error: {}", e),
        )
    })?;

    let repo_id = repo.id.to_string();

    let manifest = db::oci::upsert_manifest(
        &state.db,
        &repo_id,
        &digest,
        &media_type,
        &content_hash,
        body.len() as i64,
    )
    .await
    .map_err(|e| {
        tracing::error!("failed to store manifest: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal server error".into(),
        )
    })?;

    // If the reference is a tag (not a digest), create/update the tag
    if let Some(tag) = tag {
        db::oci::put_tag(&state.db, &repo_id, tag, &manifest.id)
            .await
            .map_err(|e| {
                tracing::error!("failed to create tag: {}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal server error".into(),
                )
            })?;
    }

    let mut resp_headers = HeaderMap::new();
    resp_headers.insert(
        header::LOCATION,
        format!("/v2/{}/{}/manifests/{}", owner, name, digest)
            .parse()
            .unwrap(),
    );
    resp_headers.insert("docker-content-digest", digest.parse().unwrap());

    Ok((StatusCode::CREATED, resp_headers))
}

async fn oci_delete_manifest(
    State(state): State<AppState>,
    Path((owner, name, reference)): Path<(String, String, String)>,
    OciUser(user): OciUser,
) -> Result<StatusCode, (StatusCode, String)> {
    let user = require_user(user)?;
    let (repo, owner_user) = resolve_repo_authed(&state, &owner, &name, &user).await?;
    require_role(&state, &repo, &owner_user, &user, CollaboratorRole::Write).await?;

    let manifest = resolve_manifest(&state, &repo.id.to_string(), &reference).await?;

    db::oci::delete_manifest(&state.db, &repo.id.to_string(), &manifest.digest)
        .await
        .map_err(|e| {
            tracing::error!("failed to delete manifest: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal server error".into(),
            )
        })?;

    if let Err(e) =
        delta_registry::store::release_blob(&state.db, &state.blob_store, &manifest.content_hash)
            .await
    {
        tracing::warn!("failed to release OCI manifest blob: {}", e);
    }
    Ok(StatusCode::ACCEPTED)
}

// --- Tags ---

async fn list_tags(
    State(state): State<AppState>,
    Path((owner, name)): Path<(String, String)>,
    OciUser(user): OciUser,
) -> Result<Json<TagListResponse>, (StatusCode, String)> {
    let (repo, _) = resolve_readable(&state, &owner, &name, user.as_ref()).await?;
    let tags = db::oci::list_tags(&state.db, &repo.id.to_string())
        .await
        .map_err(|e| {
            tracing::error!("failed to list tags: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal server error".into(),
            )
        })?;

    Ok(Json(TagListResponse {
        name: format!("{}/{}", owner, name),
        tags,
    }))
}

#[derive(Serialize)]
struct TagListResponse {
    name: String,
    tags: Vec<String>,
}

// --- Helpers ---

async fn resolve_manifest(
    state: &AppState,
    repo_id: &str,
    reference: &str,
) -> Result<db::oci::OciManifest, (StatusCode, String)> {
    if reference.starts_with("sha256:") {
        db::oci::get_manifest_by_digest(&state.db, repo_id, reference)
            .await
            .map_err(|_| (StatusCode::NOT_FOUND, "manifest not found".into()))
    } else {
        db::oci::get_manifest_by_tag(&state.db, repo_id, reference)
            .await
            .map_err(|_| {
                (
                    StatusCode::NOT_FOUND,
                    format!("tag '{}' not found", reference),
                )
            })
    }
}
