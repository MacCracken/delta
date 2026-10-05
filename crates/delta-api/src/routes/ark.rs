//! Ark package registry routes (.ark AGNOS native packages).

use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    routing::get,
};
use delta_core::db;
use delta_registry::ark::ArkPackageMeta;
use serde::{Deserialize, Serialize};

use crate::extractors::{AuthUser, MaybeAuthUser};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/ark", get(search_packages))
        .route("/ark/{name}", get(list_versions))
        .route(
            "/ark/{name}/{version}",
            get(download_package)
                .put(publish_package)
                .delete(delete_package),
        )
        .route("/ark/{name}/{version}/meta", get(get_metadata))
        .layer(axum::extract::DefaultBodyLimit::max(
            crate::helpers::MAX_UPLOAD_BYTES,
        ))
}

#[derive(Deserialize)]
struct SearchQuery {
    #[serde(default)]
    q: String,
    #[serde(default = "default_limit")]
    limit: i64,
    #[serde(default)]
    offset: i64,
}

fn default_limit() -> i64 {
    50
}

async fn search_packages(
    State(state): State<AppState>,
    Query(query): Query<SearchQuery>,
    MaybeAuthUser(user): MaybeAuthUser,
) -> Result<Json<Vec<db::ark_package::ArkPackage>>, (StatusCode, String)> {
    if query.q.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "query parameter 'q' required".into(),
        ));
    }
    let limit = query.limit.clamp(1, 100);
    let offset = query.offset.max(0);
    let packages = db::ark_package::search(&state.db, &query.q, limit, offset)
        .await
        .map_err(|e| {
            tracing::error!("package search failed: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal server error".into(),
            )
        })?;
    Ok(Json(
        visible_packages(&state, packages, user.as_ref()).await,
    ))
}

async fn list_versions(
    State(state): State<AppState>,
    Path(name): Path<String>,
    MaybeAuthUser(user): MaybeAuthUser,
) -> Result<Json<Vec<db::ark_package::ArkPackage>>, (StatusCode, String)> {
    let packages = db::ark_package::list_versions(&state.db, &name)
        .await
        .map_err(|e| {
            tracing::error!("failed to list versions: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal server error".into(),
            )
        })?;
    Ok(Json(
        visible_packages(&state, packages, user.as_ref()).await,
    ))
}

async fn get_metadata(
    State(state): State<AppState>,
    Path((name, version)): Path<(String, String)>,
    Query(query): Query<ArchQuery>,
    MaybeAuthUser(user): MaybeAuthUser,
) -> Result<Json<db::ark_package::ArkPackage>, (StatusCode, String)> {
    let pkg = db::ark_package::get_version(&state.db, &name, &version, query.arch.as_deref())
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    if !package_visible(&state, &pkg, user.as_ref()).await {
        return Err((StatusCode::NOT_FOUND, "package not found".into()));
    }
    Ok(Json(pkg))
}

#[derive(Deserialize)]
struct ArchQuery {
    arch: Option<String>,
}

async fn download_package(
    State(state): State<AppState>,
    Path((name, version)): Path<(String, String)>,
    Query(query): Query<ArchQuery>,
    MaybeAuthUser(user): MaybeAuthUser,
) -> Result<(StatusCode, Vec<u8>), (StatusCode, String)> {
    let pkg = db::ark_package::get_version(&state.db, &name, &version, query.arch.as_deref())
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    if !package_visible(&state, &pkg, user.as_ref()).await {
        return Err((StatusCode::NOT_FOUND, "package not found".into()));
    }

    let artifact = db::artifact::get(&state.db, &pkg.artifact_id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;

    let data = state
        .blob_store
        .read(&artifact.content_hash)
        .map_err(|e| (StatusCode::NOT_FOUND, format!("blob not found: {}", e)))?;

    let _ = db::artifact::increment_download(&state.db, &artifact.id).await;

    Ok((StatusCode::OK, data))
}

/// Publish an ark package. Metadata via `X-Ark-Meta` JSON header, binary body.
async fn publish_package(
    State(state): State<AppState>,
    Path((name, version)): Path<(String, String)>,
    AuthUser(user): AuthUser,
    headers: HeaderMap,
    body: Bytes,
) -> Result<(StatusCode, Json<PublishResponse>), (StatusCode, String)> {
    const MAX_PACKAGE_SIZE: usize = 100 * 1024 * 1024;
    if body.len() > MAX_PACKAGE_SIZE {
        return Err((StatusCode::PAYLOAD_TOO_LARGE, "package too large".into()));
    }

    // Parse metadata from header
    let meta_header = headers
        .get("x-ark-meta")
        .and_then(|v| v.to_str().ok())
        .ok_or((StatusCode::BAD_REQUEST, "missing X-Ark-Meta header".into()))?;

    let meta: ArkPackageMeta = serde_json::from_str(meta_header)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("invalid metadata: {}", e)))?;

    // Validate path matches metadata
    if meta.name != name || meta.version != version {
        return Err((
            StatusCode::BAD_REQUEST,
            "metadata name/version must match URL path".into(),
        ));
    }

    // Validate package name (ASCII only: no look-alike Unicode names),
    // version, and architecture
    if !is_valid_token(&name, 128, &['-', '_']) {
        return Err((StatusCode::BAD_REQUEST, "invalid package name".into()));
    }
    if !is_valid_token(&version, 64, &['.', '-', '_', '+']) {
        return Err((StatusCode::BAD_REQUEST, "invalid package version".into()));
    }
    if !is_valid_token(&meta.arch, 32, &['-', '_']) {
        return Err((
            StatusCode::BAD_REQUEST,
            "invalid package architecture".into(),
        ));
    }

    // A package name belongs to the user who first published it; nobody
    // else may publish versions (or architectures) under it.
    let existing = db::ark_package::list_versions(&state.db, &name)
        .await
        .map_err(|e| {
            tracing::error!("failed to list versions: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal server error".into(),
            )
        })?;
    let user_id = user.id.to_string();
    if !existing.is_empty() && !existing.iter().any(|p| p.publisher_id == user_id) {
        return Err((
            StatusCode::FORBIDDEN,
            "package name is owned by another publisher".into(),
        ));
    }

    // User needs at least one repo to publish to — use first repo they own
    // (in practice, the repo is implicitly the publishing namespace)
    let repos = db::repo::list_by_owner(&state.db, &user.id.to_string())
        .await
        .map_err(|e| {
            tracing::error!("failed to list repos: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal server error".into(),
            )
        })?;

    // The package takes the visibility of this repository; prefer a public one.
    let repo = repos
        .iter()
        .find(|r| r.visibility == delta_core::models::repo::Visibility::Public)
        .or(repos.first())
        .ok_or((
            StatusCode::BAD_REQUEST,
            "you must have at least one repository to publish packages".into(),
        ))?;

    // Store blob
    let content_hash = state.blob_store.store(&body).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("storage error: {}", e),
        )
    })?;

    // Create artifact record
    let artifact = db::artifact::create(
        &state.db,
        &db::artifact::CreateArtifactParams {
            repo_id: &repo.id.to_string(),
            pipeline_id: None,
            name: &name,
            version: Some(&version),
            artifact_type: "ark_package",
            content_hash: &content_hash,
            size_bytes: body.len() as i64,
            metadata: Some(meta_header),
        },
    )
    .await
    .map_err(|e| {
        tracing::error!("failed to create artifact: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal server error".into(),
        )
    })?;

    // Create ark package record
    let deps_json = serde_json::to_string(&meta.dependencies).ok();
    let provides_json = serde_json::to_string(&meta.provides).ok();

    let pkg = db::ark_package::publish(
        &state.db,
        &db::ark_package::PublishParams {
            artifact_id: &artifact.id,
            repo_id: &repo.id.to_string(),
            publisher_id: &user.id.to_string(),
            package_name: &name,
            version: &version,
            arch: &meta.arch,
            description: meta.description.as_deref(),
            dependencies: deps_json.as_deref(),
            provides: provides_json.as_deref(),
        },
    )
    .await
    .map_err(|e| (StatusCode::CONFLICT, e.to_string()))?;

    Ok((
        StatusCode::CREATED,
        Json(PublishResponse {
            package: pkg,
            artifact_id: artifact.id,
        }),
    ))
}

/// Whether `user` (or an anonymous caller) may see a package: packages
/// share the visibility of the repository they were published to.
async fn package_visible(
    state: &AppState,
    pkg: &db::ark_package::ArkPackage,
    user: Option<&delta_core::models::user::User>,
) -> bool {
    let Ok(repo) = db::repo::get_by_id(&state.db, &pkg.repo_id).await else {
        return false;
    };
    if repo.visibility == delta_core::models::repo::Visibility::Public {
        return true;
    }
    let Some(user) = user else {
        return false;
    };
    if repo.owner == user.id.to_string() {
        return true;
    }
    db::collaborator::get_role(&state.db, &pkg.repo_id, &user.id.to_string())
        .await
        .ok()
        .flatten()
        .is_some()
}

/// Keep only the packages the caller may see.
async fn visible_packages(
    state: &AppState,
    packages: Vec<db::ark_package::ArkPackage>,
    user: Option<&delta_core::models::user::User>,
) -> Vec<db::ark_package::ArkPackage> {
    let mut visible = Vec::with_capacity(packages.len());
    for pkg in packages {
        if package_visible(state, &pkg, user).await {
            visible.push(pkg);
        }
    }
    visible
}

fn is_valid_token(value: &str, max_len: usize, extra: &[char]) -> bool {
    !value.is_empty()
        && value.len() <= max_len
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || extra.contains(&c))
}

#[derive(Serialize)]
struct PublishResponse {
    package: db::ark_package::ArkPackage,
    artifact_id: String,
}

async fn delete_package(
    State(state): State<AppState>,
    Path((name, version)): Path<(String, String)>,
    Query(query): Query<ArchQuery>,
    AuthUser(user): AuthUser,
) -> Result<StatusCode, (StatusCode, String)> {
    let pkg = db::ark_package::get_version(&state.db, &name, &version, query.arch.as_deref())
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;

    if pkg.publisher_id != user.id.to_string() {
        return Err((StatusCode::FORBIDDEN, "not the publisher".into()));
    }

    // Delete the ark package record (artifact is left for cleanup)
    db::ark_package::delete(&state.db, &pkg.id)
        .await
        .map_err(|e| {
            tracing::error!("failed to delete package: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal server error".into(),
            )
        })?;

    // Also delete the artifact and blob
    if let Ok(artifact) = db::artifact::get(&state.db, &pkg.artifact_id).await
        && db::artifact::delete(&state.db, &artifact.id).await.is_ok()
        && let Err(e) = delta_registry::store::release_blob(
            &state.db,
            &state.blob_store,
            &artifact.content_hash,
        )
        .await
    {
        tracing::warn!("failed to release package blob: {}", e);
    }

    Ok(StatusCode::NO_CONTENT)
}
