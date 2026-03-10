use axum::{Json, Router, extract::State, http::StatusCode, routing::post};
use serde::{Deserialize, Serialize};

use crate::auth;
use crate::extractors::AuthUser;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/register", post(register))
        .route("/login", post(login))
        .route("/tokens", post(create_token))
}

#[derive(Deserialize)]
struct RegisterRequest {
    username: String,
    email: String,
    password: String,
    #[serde(default)]
    is_agent: bool,
}

#[derive(Serialize)]
struct AuthResponse {
    user: UserResponse,
    token: String,
}

#[derive(Serialize)]
struct UserResponse {
    id: String,
    username: String,
    email: String,
    is_agent: bool,
}

async fn register(
    State(state): State<AppState>,
    Json(req): Json<RegisterRequest>,
) -> Result<(StatusCode, Json<AuthResponse>), (StatusCode, String)> {
    // Validate username: 1-39 chars, alphanumeric and hyphens, no leading hyphen
    if req.username.is_empty()
        || req.username.len() > 39
        || req.username.starts_with('-')
        || !req
            .username
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "username must be 1-39 alphanumeric characters or hyphens, not starting with '-'"
                .into(),
        ));
    }
    if req.password.len() < 8 {
        return Err((
            StatusCode::BAD_REQUEST,
            "password must be at least 8 characters".into(),
        ));
    }
    let user = auth::register(
        &state.db,
        &req.username,
        &req.email,
        &req.password,
        req.is_agent,
    )
    .await
    .map_err(|e| (StatusCode::CONFLICT, e.to_string()))?;

    let (raw_token, token_hash) = auth::generate_token().map_err(|e| {
        tracing::error!("internal error: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal server error".into(),
        )
    })?;
    let expires_at = auth::compute_expiry(state.config.auth.token_expiry_secs);
    delta_core::db::user::create_token(
        &state.db,
        &user.id.to_string(),
        "initial",
        &token_hash,
        "*",
        expires_at.as_deref(),
    )
    .await
    .map_err(|e| {
        tracing::error!("internal error: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal server error".into(),
        )
    })?;

    Ok((
        StatusCode::CREATED,
        Json(AuthResponse {
            user: UserResponse {
                id: user.id.to_string(),
                username: user.username,
                email: user.email,
                is_agent: user.is_agent,
            },
            token: raw_token,
        }),
    ))
}

#[derive(Deserialize)]
struct LoginRequest {
    username: String,
    password: String,
}

async fn login(
    State(state): State<AppState>,
    Json(req): Json<LoginRequest>,
) -> Result<Json<AuthResponse>, (StatusCode, String)> {
    let (user, token) = auth::login(
        &state.db,
        &req.username,
        &req.password,
        state.config.auth.token_expiry_secs,
    )
    .await
    .map_err(|e| (StatusCode::UNAUTHORIZED, e.to_string()))?;

    Ok(Json(AuthResponse {
        user: UserResponse {
            id: user.id.to_string(),
            username: user.username,
            email: user.email,
            is_agent: user.is_agent,
        },
        token,
    }))
}

#[derive(Deserialize)]
struct CreateTokenRequest {
    name: String,
    #[serde(default = "default_scopes")]
    scopes: String,
}

fn default_scopes() -> String {
    "*".to_string()
}

#[derive(Serialize)]
struct TokenResponse {
    id: String,
    token: String,
}

async fn create_token(
    State(state): State<AppState>,
    AuthUser(user): AuthUser,
    Json(req): Json<CreateTokenRequest>,
) -> Result<(StatusCode, Json<TokenResponse>), (StatusCode, String)> {
    let (raw_token, token_hash) = auth::generate_token().map_err(|e| {
        tracing::error!("internal error: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal server error".into(),
        )
    })?;
    let expires_at = auth::compute_expiry(state.config.auth.token_expiry_secs);
    let id = delta_core::db::user::create_token(
        &state.db,
        &user.id.to_string(),
        &req.name,
        &token_hash,
        &req.scopes,
        expires_at.as_deref(),
    )
    .await
    .map_err(|e| {
        tracing::error!("internal error: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal server error".into(),
        )
    })?;

    Ok((
        StatusCode::CREATED,
        Json(TokenResponse {
            id,
            token: raw_token,
        }),
    ))
}
