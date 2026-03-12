//! Signing key management routes.

use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::get,
};
use delta_core::db;
use serde::Deserialize;

use crate::extractors::AuthUser;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/signing-keys", get(list_keys).post(add_key))
        .route("/signing-keys/{key_id}", get(get_key).delete(delete_key))
}

#[derive(Deserialize)]
struct AddKeyRequest {
    name: String,
    public_key_hex: String,
}

async fn add_key(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Json(req): Json<AddKeyRequest>,
) -> Result<(StatusCode, Json<db::signing::SigningKey>), (StatusCode, String)> {
    if req.name.is_empty() || req.name.len() > 128 {
        return Err((
            StatusCode::BAD_REQUEST,
            "name must be 1-128 characters".into(),
        ));
    }

    // Validate hex and key length (32 bytes = 64 hex chars)
    if req.public_key_hex.len() != 64 || !req.public_key_hex.chars().all(|c| c.is_ascii_hexdigit())
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "public_key_hex must be 64 hex characters (32 bytes ed25519)".into(),
        ));
    }

    let key = db::signing::add_signing_key(
        &state.db,
        &user.id.to_string(),
        &req.name,
        &req.public_key_hex,
    )
    .await
    .map_err(|e| (StatusCode::CONFLICT, e.to_string()))?;

    Ok((StatusCode::CREATED, Json(key)))
}

async fn list_keys(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
) -> Result<Json<Vec<db::signing::SigningKey>>, (StatusCode, String)> {
    let keys = db::signing::list_signing_keys(&state.db, &user.id.to_string())
        .await
        .map_err(|e| {
            tracing::error!("failed to list signing keys: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal server error".into(),
            )
        })?;
    Ok(Json(keys))
}

async fn get_key(
    State(state): State<AppState>,
    Path(key_id): Path<String>,
    AuthUser(user): AuthUser,
) -> Result<Json<db::signing::SigningKey>, (StatusCode, String)> {
    let key = db::signing::get_signing_key(&state.db, &key_id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    if key.user_id != user.id.to_string() {
        return Err((StatusCode::NOT_FOUND, "signing key not found".into()));
    }
    Ok(Json(key))
}

async fn delete_key(
    State(state): State<AppState>,
    Path(key_id): Path<String>,
    AuthUser(user): AuthUser,
) -> Result<StatusCode, (StatusCode, String)> {
    db::signing::delete_signing_key(&state.db, &key_id, &user.id.to_string())
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}
