use axum::{
    extract::FromRequestParts,
    http::{StatusCode, request::Parts},
};
use delta_core::models::user::User;

use crate::state::AppState;
use crate::auth;

/// Extractor that authenticates the request via Bearer token.
/// Rejects with 401 if no valid token is provided.
pub struct AuthUser(pub User);

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> std::result::Result<Self, Self::Rejection> {
        let header = parts
            .headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .ok_or((StatusCode::UNAUTHORIZED, "missing authorization header"))?;

        let token = header
            .strip_prefix("Bearer ")
            .ok_or((StatusCode::UNAUTHORIZED, "invalid authorization format"))?;

        let user = auth::authenticate_token(&state.db, token)
            .await
            .map_err(|_| (StatusCode::UNAUTHORIZED, "invalid or expired token"))?;

        Ok(AuthUser(user))
    }
}

/// Optional auth — returns None if no token provided, errors if token is invalid.
pub struct MaybeAuthUser(pub Option<User>);

impl FromRequestParts<AppState> for MaybeAuthUser {
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> std::result::Result<Self, Self::Rejection> {
        let header = parts
            .headers
            .get("authorization")
            .and_then(|v| v.to_str().ok());

        match header {
            None => Ok(MaybeAuthUser(None)),
            Some(h) => {
                let token = h
                    .strip_prefix("Bearer ")
                    .ok_or((StatusCode::UNAUTHORIZED, "invalid authorization format"))?;

                let user = auth::authenticate_token(&state.db, token)
                    .await
                    .map_err(|_| (StatusCode::UNAUTHORIZED, "invalid or expired token"))?;

                Ok(MaybeAuthUser(Some(user)))
            }
        }
    }
}
