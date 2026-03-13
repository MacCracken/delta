//! Federation protocol — instance-to-instance communication.

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
};
use delta_core::db;
use serde::Deserialize;

use crate::extractors::AuthUser;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        // Instance registry (admin)
        .route("/instances", get(list_instances).post(add_instance))
        .route(
            "/instances/{instance_id}",
            get(get_instance).delete(remove_instance),
        )
        .route("/instances/{instance_id}/trust", post(update_trust))
        // Federation info endpoint (public, called by remote instances)
        .route("/info", get(instance_info))
        // Remote repo browsing
        .route("/instances/{instance_id}/repos", get(list_remote_repos))
        // Create a mirror from a federated instance
        .route("/mirror", post(create_mirror))
}

async fn instance_info(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "name": state.config.federation.instance_name,
        "url": state.config.federation.instance_url,
        "version": env!("CARGO_PKG_VERSION"),
        "federation_enabled": state.config.federation.enabled,
    }))
}

#[derive(Deserialize)]
struct AddInstanceRequest {
    url: String,
    name: Option<String>,
    public_key: Option<String>,
    trusted: Option<bool>,
}

async fn add_instance(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
    Json(req): Json<AddInstanceRequest>,
) -> Result<(StatusCode, Json<db::federation::FederationInstance>), (StatusCode, String)> {
    if req.url.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "url must not be empty".into()));
    }

    let instance = db::federation::add_instance(
        &state.db,
        &req.url,
        req.name.as_deref(),
        req.public_key.as_deref(),
        req.trusted.unwrap_or(false),
    )
    .await
    .map_err(|e| {
        tracing::error!("failed to add federation instance: {}", e);
        match e {
            delta_core::DeltaError::Conflict(msg) => (StatusCode::CONFLICT, msg),
            _ => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal server error".into(),
            ),
        }
    })?;

    Ok((StatusCode::CREATED, Json(instance)))
}

async fn list_instances(
    State(state): State<AppState>,
    AuthUser(_user): AuthUser,
) -> Result<Json<Vec<db::federation::FederationInstance>>, (StatusCode, String)> {
    let instances = db::federation::list_instances(&state.db)
        .await
        .map_err(|e| {
            tracing::error!("failed to list federation instances: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal server error".into(),
            )
        })?;

    Ok(Json(instances))
}

async fn get_instance(
    State(state): State<AppState>,
    Path(instance_id): Path<String>,
    AuthUser(_user): AuthUser,
) -> Result<Json<db::federation::FederationInstance>, (StatusCode, String)> {
    let instance = db::federation::get_instance(&state.db, &instance_id)
        .await
        .map_err(|e| match e {
            delta_core::DeltaError::NotFound(msg) => (StatusCode::NOT_FOUND, msg),
            _ => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal server error".into(),
            ),
        })?;

    Ok(Json(instance))
}

async fn remove_instance(
    State(state): State<AppState>,
    Path(instance_id): Path<String>,
    AuthUser(_user): AuthUser,
) -> Result<StatusCode, (StatusCode, String)> {
    db::federation::delete_instance(&state.db, &instance_id)
        .await
        .map_err(|e| match e {
            delta_core::DeltaError::NotFound(msg) => (StatusCode::NOT_FOUND, msg),
            _ => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal server error".into(),
            ),
        })?;

    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct UpdateTrustRequest {
    trusted: bool,
}

async fn update_trust(
    State(state): State<AppState>,
    Path(instance_id): Path<String>,
    AuthUser(_user): AuthUser,
    Json(req): Json<UpdateTrustRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    db::federation::update_trust(&state.db, &instance_id, req.trusted)
        .await
        .map_err(|e| match e {
            delta_core::DeltaError::NotFound(msg) => (StatusCode::NOT_FOUND, msg),
            _ => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal server error".into(),
            ),
        })?;

    Ok(StatusCode::NO_CONTENT)
}

async fn list_remote_repos(
    State(state): State<AppState>,
    Path(instance_id): Path<String>,
    AuthUser(_user): AuthUser,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let instance = db::federation::get_instance(&state.db, &instance_id)
        .await
        .map_err(|e| match e {
            delta_core::DeltaError::NotFound(msg) => (StatusCode::NOT_FOUND, msg),
            _ => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal server error".into(),
            ),
        })?;

    // Update last_seen timestamp
    let _ = db::federation::update_last_seen(&state.db, &instance_id).await;

    let timeout = std::time::Duration::from_secs(state.config.federation.timeout_secs);
    let client = reqwest::Client::builder()
        .timeout(timeout)
        .build()
        .map_err(|e| {
            tracing::error!("failed to build HTTP client: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal server error".into(),
            )
        })?;

    let url = format!("{}/api/v1/repos", instance.url.trim_end_matches('/'));
    let resp = client.get(&url).send().await.map_err(|e| {
        tracing::error!("failed to fetch remote repos from {}: {}", url, e);
        (
            StatusCode::BAD_GATEWAY,
            format!("failed to reach remote instance: {}", e),
        )
    })?;

    let body: serde_json::Value = resp.json().await.map_err(|e| {
        tracing::error!("failed to parse remote repos response: {}", e);
        (
            StatusCode::BAD_GATEWAY,
            "invalid response from remote instance".into(),
        )
    })?;

    Ok(Json(body))
}

#[derive(Deserialize)]
struct CreateMirrorRequest {
    instance_id: String,
    owner: String,
    name: String,
    local_name: Option<String>,
}

async fn create_mirror(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Json(req): Json<CreateMirrorRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, String)> {
    let instance = db::federation::get_instance(&state.db, &req.instance_id)
        .await
        .map_err(|e| match e {
            delta_core::DeltaError::NotFound(msg) => (StatusCode::NOT_FOUND, msg),
            _ => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal server error".into(),
            ),
        })?;

    let remote_url = format!(
        "{}/{}/{}.git",
        instance.url.trim_end_matches('/'),
        req.owner,
        req.name
    );
    let local_name = req.local_name.as_deref().unwrap_or(&req.name);
    let user_id = user.id.to_string();

    // Create the mirror record in DB
    let repo = db::repo::create_mirror(
        &state.db,
        &user_id,
        local_name,
        Some(&format!(
            "Mirror of {}/{} from {}",
            req.owner, req.name, instance.url
        )),
        &remote_url,
        Some(&instance.id),
    )
    .await
    .map_err(|e| match e {
        delta_core::DeltaError::Conflict(msg) => (StatusCode::CONFLICT, msg),
        _ => {
            tracing::error!("failed to create mirror repo: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal server error".into(),
            )
        }
    })?;

    // Clone the remote repo as a bare mirror
    let repo_path = state
        .config
        .storage
        .repos_dir
        .join(&user_id)
        .join(format!("{}.git", local_name));

    let output = tokio::process::Command::new("git")
        .args(["clone", "--mirror", &remote_url])
        .arg(&repo_path)
        .output()
        .await
        .map_err(|e| {
            tracing::error!("failed to run git clone --mirror: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "failed to clone remote repository".into(),
            )
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::error!("git clone --mirror failed: {}", stderr);
        // Clean up the DB record since clone failed
        let _ = db::repo::delete(&state.db, &repo.id.to_string()).await;
        return Err((
            StatusCode::BAD_GATEWAY,
            format!("failed to clone from remote: {}", stderr),
        ));
    }

    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({
            "id": repo.id,
            "name": repo.name,
            "owner": repo.owner,
            "mirror_url": repo.mirror_url,
            "is_mirror": repo.is_mirror,
            "federation_instance_id": repo.federation_instance_id,
            "created_at": repo.created_at,
        })),
    ))
}
