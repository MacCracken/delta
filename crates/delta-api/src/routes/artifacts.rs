//! Phase 5: Artifact registry and release API routes.

use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    routing::{get, put},
};
use delta_core::db;
use delta_registry::retention;
use serde::{Deserialize, Serialize};

use delta_core::models::collaborator::CollaboratorRole;

use crate::extractors::AuthUser;
use crate::helpers::{require_role, resolve_repo_authed};
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
            "/{owner}/{name}/artifacts/{artifact_id}/stats",
            get(artifact_stats),
        )
        .route(
            "/{owner}/{name}/artifacts/{artifact_id}/signatures",
            get(list_signatures).post(add_signature),
        )
        .route(
            "/{owner}/{name}/artifacts/{artifact_id}/verify",
            get(verify_signatures),
        )
        .route(
            "/{owner}/{name}/artifacts/retention",
            get(get_retention).put(set_retention),
        )
        .route(
            "/{owner}/{name}/artifacts/cleanup",
            put(run_cleanup),
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
    require_role(&state, &repo, &owner_user, &user, CollaboratorRole::Write).await?;

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
    headers: HeaderMap,
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

    // Record download event with user-agent tracking
    let user_agent = headers
        .get("user-agent")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let _ = db::download_stats::record_download(
        &state.db,
        &artifact_id,
        Some(&user.id.to_string()),
        user_agent.as_deref(),
        None,
    )
    .await;

    // Audit log
    let _ = db::audit::log(
        &state.db,
        Some(&user.id.to_string()),
        "download",
        "artifact",
        Some(&artifact_id),
        None,
        None,
    )
    .await;

    Ok((StatusCode::OK, data))
}

async fn delete_artifact(
    State(state): State<AppState>,
    Path((owner, name, artifact_id)): Path<(String, String, String)>,
    AuthUser(user): AuthUser,
) -> Result<StatusCode, (StatusCode, String)> {
    let (repo, owner_user) = resolve_repo_authed(&state, &owner, &name, &user).await?;
    require_role(&state, &repo, &owner_user, &user, CollaboratorRole::Write).await?;
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

// --- Download Stats ---

#[derive(Deserialize)]
struct StatsQuery {
    #[serde(default = "default_days")]
    days: i64,
}

fn default_days() -> i64 {
    30
}

async fn artifact_stats(
    State(state): State<AppState>,
    Path((owner, name, artifact_id)): Path<(String, String, String)>,
    AuthUser(user): AuthUser,
    Query(query): Query<StatsQuery>,
) -> Result<Json<ArtifactStatsResponse>, (StatusCode, String)> {
    let (repo, _) = resolve_repo_authed(&state, &owner, &name, &user).await?;
    let artifact = db::artifact::get(&state.db, &artifact_id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    if artifact.repo_id != repo.id.to_string() {
        return Err((StatusCode::NOT_FOUND, "artifact not found".into()));
    }

    let daily = db::download_stats::get_daily_counts(&state.db, &artifact_id, query.days)
        .await
        .map_err(|e| {
            tracing::error!("failed to get stats: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal server error".into(),
            )
        })?;

    Ok(Json(ArtifactStatsResponse {
        artifact_id: artifact.id,
        total_downloads: artifact.download_count,
        daily_counts: daily,
    }))
}

#[derive(Serialize)]
struct ArtifactStatsResponse {
    artifact_id: String,
    total_downloads: i64,
    daily_counts: Vec<db::download_stats::DailyCount>,
}

// --- Signing ---

#[derive(Deserialize)]
struct AddSignatureRequest {
    key_id: String,
    signature: String,
}

async fn add_signature(
    State(state): State<AppState>,
    Path((owner, name, artifact_id)): Path<(String, String, String)>,
    AuthUser(user): AuthUser,
    Json(req): Json<AddSignatureRequest>,
) -> Result<(StatusCode, Json<db::signing::ArtifactSignature>), (StatusCode, String)> {
    let (repo, _) = resolve_repo_authed(&state, &owner, &name, &user).await?;
    let artifact = db::artifact::get(&state.db, &artifact_id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    if artifact.repo_id != repo.id.to_string() {
        return Err((StatusCode::NOT_FOUND, "artifact not found".into()));
    }

    // Verify the key belongs to the user
    let key = db::signing::get_signing_key(&state.db, &req.key_id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    if key.user_id != user.id.to_string() {
        return Err((StatusCode::FORBIDDEN, "signing key does not belong to you".into()));
    }

    // Verify the signature is valid before storing
    let valid = delta_registry::signing::verify_signature(
        &key.public_key_hex,
        &artifact.content_hash,
        &req.signature,
    )
    .map_err(|e| (StatusCode::BAD_REQUEST, e))?;

    if !valid {
        return Err((StatusCode::BAD_REQUEST, "signature verification failed".into()));
    }

    let sig = db::signing::add_signature(&state.db, &artifact_id, &req.key_id, &req.signature)
        .await
        .map_err(|e| (StatusCode::CONFLICT, e.to_string()))?;

    Ok((StatusCode::CREATED, Json(sig)))
}

async fn list_signatures(
    State(state): State<AppState>,
    Path((owner, name, artifact_id)): Path<(String, String, String)>,
    AuthUser(user): AuthUser,
) -> Result<Json<Vec<db::signing::ArtifactSignature>>, (StatusCode, String)> {
    let (repo, _) = resolve_repo_authed(&state, &owner, &name, &user).await?;
    let artifact = db::artifact::get(&state.db, &artifact_id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    if artifact.repo_id != repo.id.to_string() {
        return Err((StatusCode::NOT_FOUND, "artifact not found".into()));
    }

    let sigs = db::signing::get_signatures(&state.db, &artifact_id)
        .await
        .map_err(|e| {
            tracing::error!("failed to list signatures: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal server error".into(),
            )
        })?;

    Ok(Json(sigs))
}

async fn verify_signatures(
    State(state): State<AppState>,
    Path((owner, name, artifact_id)): Path<(String, String, String)>,
    AuthUser(user): AuthUser,
) -> Result<Json<Vec<delta_registry::signing::VerificationResult>>, (StatusCode, String)> {
    let (repo, _) = resolve_repo_authed(&state, &owner, &name, &user).await?;
    let artifact = db::artifact::get(&state.db, &artifact_id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    if artifact.repo_id != repo.id.to_string() {
        return Err((StatusCode::NOT_FOUND, "artifact not found".into()));
    }

    let sigs = db::signing::get_signatures(&state.db, &artifact_id)
        .await
        .map_err(|e| {
            tracing::error!("failed to get signatures: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal server error".into(),
            )
        })?;

    let mut results = Vec::new();
    for sig in sigs {
        let key = db::signing::get_signing_key(&state.db, &sig.signer_key_id).await;
        let (valid, key_name) = match key {
            Ok(k) => {
                let v = delta_registry::signing::verify_signature(
                    &k.public_key_hex,
                    &artifact.content_hash,
                    &sig.signature_hex,
                )
                .unwrap_or(false);
                (v, k.name)
            }
            Err(_) => (false, "unknown".to_string()),
        };
        results.push(delta_registry::signing::VerificationResult {
            key_id: sig.signer_key_id,
            key_name,
            valid,
        });
    }

    Ok(Json(results))
}

// --- Retention ---

#[derive(Deserialize)]
struct SetRetentionRequest {
    max_age_days: Option<i64>,
    max_count: Option<i64>,
    max_total_bytes: Option<i64>,
}

async fn get_retention(
    State(state): State<AppState>,
    Path((owner, name)): Path<(String, String)>,
    AuthUser(user): AuthUser,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let (repo, _) = resolve_repo_authed(&state, &owner, &name, &user).await?;
    let policy = db::retention::get_policy(&state.db, &repo.id.to_string())
        .await
        .map_err(|e| {
            tracing::error!("failed to get retention policy: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal server error".into(),
            )
        })?;
    match policy {
        Some(p) => Ok(Json(serde_json::to_value(p).unwrap())),
        None => Ok(Json(serde_json::json!({"policy": null}))),
    }
}

async fn set_retention(
    State(state): State<AppState>,
    Path((owner, name)): Path<(String, String)>,
    AuthUser(user): AuthUser,
    Json(req): Json<SetRetentionRequest>,
) -> Result<Json<db::retention::RetentionPolicy>, (StatusCode, String)> {
    let (repo, owner_user) = resolve_repo_authed(&state, &owner, &name, &user).await?;
    require_role(&state, &repo, &owner_user, &user, CollaboratorRole::Admin).await?;

    let policy = db::retention::set_policy(
        &state.db,
        &db::retention::SetPolicyParams {
            repo_id: &repo.id.to_string(),
            max_age_days: req.max_age_days,
            max_count: req.max_count,
            max_total_bytes: req.max_total_bytes,
        },
    )
    .await
    .map_err(|e| {
        tracing::error!("failed to set retention policy: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal server error".into(),
        )
    })?;

    Ok(Json(policy))
}

async fn run_cleanup(
    State(state): State<AppState>,
    Path((owner, name)): Path<(String, String)>,
    AuthUser(user): AuthUser,
) -> Result<Json<retention::CleanupReport>, (StatusCode, String)> {
    let (repo, owner_user) = resolve_repo_authed(&state, &owner, &name, &user).await?;
    require_role(&state, &repo, &owner_user, &user, CollaboratorRole::Admin).await?;

    let repo_id = repo.id.to_string();

    // Get per-repo policy, falling back to global config
    let policy = db::retention::get_policy(&state.db, &repo_id)
        .await
        .map_err(|e| {
            tracing::error!("failed to get retention policy: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal server error".into(),
            )
        })?;

    let (max_age, max_count, max_bytes) = match policy {
        Some(p) => (p.max_age_days, p.max_count, p.max_total_bytes),
        None => (
            state.config.registry.max_artifact_age_days.map(|d| d as i64),
            state.config.registry.max_artifacts_per_repo.map(|c| c as i64),
            state.config.registry.max_total_bytes_per_repo.map(|b| b as i64),
        ),
    };

    let report = retention::cleanup_repo(
        &state.db,
        &state.blob_store,
        &repo_id,
        max_age,
        max_count,
        max_bytes,
    )
    .await
    .map_err(|e| {
        tracing::error!("cleanup failed: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal server error".into(),
        )
    })?;

    Ok(Json(report))
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
    require_role(&state, &repo, &owner_user, &user, CollaboratorRole::Write).await?;
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
    require_role(&state, &repo, &owner_user, &user, CollaboratorRole::Write).await?;
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
