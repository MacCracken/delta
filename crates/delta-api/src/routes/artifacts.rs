//! Phase 5: Artifact registry and release API routes.

use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path, State},
    http::StatusCode,
    routing::get,
};
use delta_core::db;
use serde::Deserialize;

use crate::extractors::AuthUser;
use crate::helpers::resolve_repo_authed;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/{owner}/{name}/artifacts",
            get(list_artifacts).post(upload_artifact),
        )
        .route(
            "/{owner}/{name}/artifacts/{artifact_id}",
            get(get_artifact).delete(delete_artifact),
        )
        .route(
            "/{owner}/{name}/artifacts/{artifact_id}/download",
            get(download_artifact),
        )
        .route(
            "/{owner}/{name}/releases",
            get(list_releases).post(create_release),
        )
        .route(
            "/{owner}/{name}/releases/{tag}",
            get(get_release).delete(delete_release),
        )
}

async fn list_artifacts(
    State(state): State<AppState>,
    Path((owner, name)): Path<(String, String)>,
    AuthUser(user): AuthUser,
) -> Result<Json<Vec<db::artifact::Artifact>>, (StatusCode, String)> {
    let (repo, _) = resolve_repo_authed(&state, &owner, &name, &user).await?;
    let artifacts = db::artifact::list_for_repo(&state.db, &repo.id.to_string())
        .await
        .map_err(|e| {
            tracing::error!("failed to list artifacts: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal server error".into(),
            )
        })?;
    Ok(Json(artifacts))
}

async fn upload_artifact(
    State(state): State<AppState>,
    Path((owner, name)): Path<(String, String)>,
    AuthUser(user): AuthUser,
    body: Bytes,
) -> Result<(StatusCode, Json<db::artifact::Artifact>), (StatusCode, String)> {
    // Limit artifact upload size to 100 MB
    const MAX_ARTIFACT_SIZE: usize = 100 * 1024 * 1024;
    if body.len() > MAX_ARTIFACT_SIZE {
        return Err((
            StatusCode::PAYLOAD_TOO_LARGE,
            format!(
                "artifact size exceeds maximum of {} bytes",
                MAX_ARTIFACT_SIZE
            ),
        ));
    }

    let (repo, owner_user) = resolve_repo_authed(&state, &owner, &name, &user).await?;
    if user.id != owner_user.id {
        return Err((StatusCode::FORBIDDEN, "not the repository owner".into()));
    }

    // Store in blob store
    let content_hash = state.blob_store.store(&body).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("storage error: {}", e),
        )
    })?;

    let artifact = db::artifact::create(
        &state.db,
        &db::artifact::CreateArtifactParams {
            repo_id: &repo.id.to_string(),
            pipeline_id: None,
            name: "upload",
            version: None,
            artifact_type: "generic",
            content_hash: &content_hash,
            size_bytes: body.len() as i64,
            metadata: None,
        },
    )
    .await
    .map_err(|e| {
        tracing::error!("failed to create artifact record: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal server error".into(),
        )
    })?;

    Ok((StatusCode::CREATED, Json(artifact)))
}

async fn get_artifact(
    State(state): State<AppState>,
    Path((owner, name, artifact_id)): Path<(String, String, String)>,
    AuthUser(user): AuthUser,
) -> Result<Json<db::artifact::Artifact>, (StatusCode, String)> {
    let (repo, _) = resolve_repo_authed(&state, &owner, &name, &user).await?;
    let artifact = db::artifact::get(&state.db, &artifact_id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    if artifact.repo_id != repo.id.to_string() {
        return Err((StatusCode::NOT_FOUND, "artifact not found".into()));
    }
    Ok(Json(artifact))
}

async fn download_artifact(
    State(state): State<AppState>,
    Path((owner, name, artifact_id)): Path<(String, String, String)>,
    AuthUser(user): AuthUser,
) -> Result<(StatusCode, Vec<u8>), (StatusCode, String)> {
    let (repo, _) = resolve_repo_authed(&state, &owner, &name, &user).await?;
    let artifact = db::artifact::get(&state.db, &artifact_id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    if artifact.repo_id != repo.id.to_string() {
        return Err((StatusCode::NOT_FOUND, "artifact not found".into()));
    }

    let data = state
        .blob_store
        .read(&artifact.content_hash)
        .map_err(|e| (StatusCode::NOT_FOUND, format!("blob not found: {}", e)))?;

    let _ = db::artifact::increment_download(&state.db, &artifact_id).await;

    Ok((StatusCode::OK, data))
}

async fn delete_artifact(
    State(state): State<AppState>,
    Path((owner, name, artifact_id)): Path<(String, String, String)>,
    AuthUser(user): AuthUser,
) -> Result<StatusCode, (StatusCode, String)> {
    let (repo, owner_user) = resolve_repo_authed(&state, &owner, &name, &user).await?;
    if user.id != owner_user.id {
        return Err((StatusCode::FORBIDDEN, "not the repository owner".into()));
    }
    let artifact = db::artifact::get(&state.db, &artifact_id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    if artifact.repo_id != repo.id.to_string() {
        return Err((StatusCode::NOT_FOUND, "artifact not found".into()));
    }

    db::artifact::delete(&state.db, &artifact_id)
        .await
        .map_err(|e| {
            tracing::error!("failed to delete artifact: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal server error".into(),
            )
        })?;

    let _ = state.blob_store.delete(&artifact.content_hash);
    Ok(StatusCode::NO_CONTENT)
}

// --- Releases ---

async fn list_releases(
    State(state): State<AppState>,
    Path((owner, name)): Path<(String, String)>,
    AuthUser(user): AuthUser,
) -> Result<Json<Vec<db::release::Release>>, (StatusCode, String)> {
    let (repo, _) = resolve_repo_authed(&state, &owner, &name, &user).await?;
    let releases = db::release::list_for_repo(&state.db, &repo.id.to_string())
        .await
        .map_err(|e| {
            tracing::error!("failed to list releases: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal server error".into(),
            )
        })?;
    Ok(Json(releases))
}

#[derive(Deserialize)]
struct CreateReleaseRequest {
    tag_name: String,
    name: String,
    body: Option<String>,
    #[serde(default)]
    is_draft: bool,
    #[serde(default)]
    is_prerelease: bool,
}

async fn create_release(
    State(state): State<AppState>,
    Path((owner, name)): Path<(String, String)>,
    AuthUser(user): AuthUser,
    Json(req): Json<CreateReleaseRequest>,
) -> Result<(StatusCode, Json<db::release::Release>), (StatusCode, String)> {
    // Validate input lengths
    if req.tag_name.is_empty() || req.tag_name.len() > 128 {
        return Err((
            StatusCode::BAD_REQUEST,
            "tag_name must be 1-128 characters".into(),
        ));
    }
    if req.name.is_empty() || req.name.len() > 256 {
        return Err((
            StatusCode::BAD_REQUEST,
            "release name must be 1-256 characters".into(),
        ));
    }
    if let Some(body) = &req.body
        && body.len() > 65536
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "release body must be at most 65536 characters".into(),
        ));
    }

    let (repo, owner_user) = resolve_repo_authed(&state, &owner, &name, &user).await?;
    if user.id != owner_user.id {
        return Err((StatusCode::FORBIDDEN, "not the repository owner".into()));
    }
    let release = db::release::create(
        &state.db,
        &db::release::CreateReleaseParams {
            repo_id: &repo.id.to_string(),
            tag_name: &req.tag_name,
            name: &req.name,
            body: req.body.as_deref(),
            is_draft: req.is_draft,
            is_prerelease: req.is_prerelease,
            author_id: &user.id.to_string(),
        },
    )
    .await
    .map_err(|e| (StatusCode::CONFLICT, e.to_string()))?;
    Ok((StatusCode::CREATED, Json(release)))
}

async fn get_release(
    State(state): State<AppState>,
    Path((owner, name, tag)): Path<(String, String, String)>,
    AuthUser(user): AuthUser,
) -> Result<Json<db::release::Release>, (StatusCode, String)> {
    let (repo, _) = resolve_repo_authed(&state, &owner, &name, &user).await?;
    let release = db::release::get_by_tag(&state.db, &repo.id.to_string(), &tag)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    Ok(Json(release))
}

async fn delete_release(
    State(state): State<AppState>,
    Path((owner, name, tag)): Path<(String, String, String)>,
    AuthUser(user): AuthUser,
) -> Result<StatusCode, (StatusCode, String)> {
    let (repo, owner_user) = resolve_repo_authed(&state, &owner, &name, &user).await?;
    if user.id != owner_user.id {
        return Err((StatusCode::FORBIDDEN, "not the repository owner".into()));
    }
    let release = db::release::get_by_tag(&state.db, &repo.id.to_string(), &tag)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    db::release::delete(&state.db, &release.id)
        .await
        .map_err(|e| {
            tracing::error!("failed to delete release: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal server error".into(),
            )
        })?;
    Ok(StatusCode::NO_CONTENT)
}

// resolve_repo is imported from crate::helpers
