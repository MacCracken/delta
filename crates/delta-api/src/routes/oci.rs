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

use crate::extractors::AuthUser;
use crate::helpers::{require_role, resolve_repo_authed};
use crate::state::AppState;

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
}

// --- Version Check ---

async fn version_check() -> (StatusCode, Json<serde_json::Value>) {
    (StatusCode::OK, Json(serde_json::json!({})))
}

// --- Blobs ---

async fn check_blob(
    State(state): State<AppState>,
    Path((owner, name, digest)): Path<(String, String, String)>,
    AuthUser(user): AuthUser,
) -> Result<(StatusCode, HeaderMap), (StatusCode, String)> {
    let (repo, _) = resolve_repo_authed(&state, &owner, &name, &user).await?;
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
    AuthUser(user): AuthUser,
) -> Result<(StatusCode, HeaderMap, Vec<u8>), (StatusCode, String)> {
    let (repo, _) = resolve_repo_authed(&state, &owner, &name, &user).await?;
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
    AuthUser(user): AuthUser,
) -> Result<StatusCode, (StatusCode, String)> {
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

    let _ = state.blob_store.delete(&blob.content_hash);
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
    AuthUser(user): AuthUser,
    Query(query): Query<UploadQuery>,
    body: Bytes,
) -> Result<(StatusCode, HeaderMap), (StatusCode, String)> {
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
    AuthUser(user): AuthUser,
    body: Bytes,
) -> Result<(StatusCode, HeaderMap), (StatusCode, String)> {
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
    AuthUser(user): AuthUser,
    Query(query): Query<UploadQuery>,
    body: Bytes,
) -> Result<(StatusCode, HeaderMap), (StatusCode, String)> {
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
    AuthUser(user): AuthUser,
) -> Result<(StatusCode, HeaderMap), (StatusCode, String)> {
    let (repo, _) = resolve_repo_authed(&state, &owner, &name, &user).await?;
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
    AuthUser(user): AuthUser,
) -> Result<(StatusCode, HeaderMap, Vec<u8>), (StatusCode, String)> {
    let (repo, _) = resolve_repo_authed(&state, &owner, &name, &user).await?;
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
    AuthUser(user): AuthUser,
    headers: HeaderMap,
    body: Bytes,
) -> Result<(StatusCode, HeaderMap), (StatusCode, String)> {
    let (repo, owner_user) = resolve_repo_authed(&state, &owner, &name, &user).await?;
    require_role(&state, &repo, &owner_user, &user, CollaboratorRole::Write).await?;

    let media_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/vnd.oci.image.manifest.v1+json")
        .to_string();

    let digest = sha256_digest(&body);

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

    // If reference is a tag (not a digest), create/update the tag
    if !reference.starts_with("sha256:") {
        db::oci::put_tag(&state.db, &repo_id, &reference, &manifest.id)
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
    AuthUser(user): AuthUser,
) -> Result<StatusCode, (StatusCode, String)> {
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

    let _ = state.blob_store.delete(&manifest.content_hash);
    Ok(StatusCode::ACCEPTED)
}

// --- Tags ---

async fn list_tags(
    State(state): State<AppState>,
    Path((owner, name)): Path<(String, String)>,
    AuthUser(user): AuthUser,
) -> Result<Json<TagListResponse>, (StatusCode, String)> {
    let (repo, _) = resolve_repo_authed(&state, &owner, &name, &user).await?;
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
