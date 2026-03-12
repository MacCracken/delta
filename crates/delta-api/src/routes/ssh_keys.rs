use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::get,
};
use delta_core::db;
use serde::{Deserialize, Serialize};

use crate::extractors::AuthUser;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_keys).post(add_key))
        .route("/{key_id}", get(get_key).delete(delete_key))
}

#[derive(Serialize)]
struct SshKeyResponse {
    id: String,
    name: String,
    fingerprint: String,
    public_key: String,
    created_at: String,
}

impl SshKeyResponse {
    fn from_key(key: delta_core::models::ssh_key::SshKey) -> Self {
        Self {
            id: key.id.to_string(),
            name: key.name,
            fingerprint: key.fingerprint,
            public_key: key.public_key,
            created_at: key.created_at.to_rfc3339(),
        }
    }
}

#[derive(Deserialize)]
struct AddKeyRequest {
    name: String,
    public_key: String,
}

/// Compute the SHA-256 fingerprint of an OpenSSH public key string.
/// Uses the same algorithm as `ssh_key::PublicKey::fingerprint()` to ensure
/// consistency with SSH server authentication.
pub fn compute_fingerprint(public_key: &str) -> Result<String, String> {
    let parsed: russh::keys::PublicKey = public_key
        .parse()
        .map_err(|e| format!("invalid SSH public key: {}", e))?;

    let fp = parsed.fingerprint(russh::keys::HashAlg::Sha256);
    Ok(fp.to_string())
}

async fn add_key(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Json(req): Json<AddKeyRequest>,
) -> Result<(StatusCode, Json<SshKeyResponse>), (StatusCode, String)> {
    let public_key = req.public_key.trim().to_string();

    if req.name.is_empty() || req.name.len() > 100 {
        return Err((StatusCode::BAD_REQUEST, "invalid key name".into()));
    }

    let fingerprint = compute_fingerprint(&public_key).map_err(|e| (StatusCode::BAD_REQUEST, e))?;

    let key = db::ssh_key::add(
        &state.db,
        &user.id.to_string(),
        &req.name,
        &public_key,
        &fingerprint,
    )
    .await
    .map_err(|e| (StatusCode::CONFLICT, e.to_string()))?;

    Ok((StatusCode::CREATED, Json(SshKeyResponse::from_key(key))))
}

async fn list_keys(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
) -> Result<Json<Vec<SshKeyResponse>>, (StatusCode, String)> {
    let keys = db::ssh_key::list_by_user(&state.db, &user.id.to_string())
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(
        keys.into_iter().map(SshKeyResponse::from_key).collect(),
    ))
}

async fn get_key(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(key_id): Path<String>,
) -> Result<Json<SshKeyResponse>, (StatusCode, String)> {
    let key = db::ssh_key::get_by_id(&state.db, &key_id)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "SSH key not found".into()))?;

    if key.user_id != user.id {
        return Err((StatusCode::NOT_FOUND, "SSH key not found".into()));
    }

    Ok(Json(SshKeyResponse::from_key(key)))
}

async fn delete_key(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Path(key_id): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    db::ssh_key::delete(&state.db, &key_id, &user.id.to_string())
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "SSH key not found".into()))?;

    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_fingerprint_ed25519() {
        let key = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl test@example";
        let fp = compute_fingerprint(key).unwrap();
        assert!(fp.starts_with("SHA256:"));
        assert!(fp.len() > 10);
    }

    #[test]
    fn test_compute_fingerprint_invalid() {
        assert!(compute_fingerprint("not a key").is_err());
        assert!(compute_fingerprint("").is_err());
    }
}
