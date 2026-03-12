use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::get,
};
use serde::{Deserialize, Serialize};

use delta_core::models::collaborator::CollaboratorRole;

use crate::extractors::AuthUser;
use crate::helpers::require_role;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/{owner}/{name}/webhooks",
            get(list_webhooks).post(create_webhook),
        )
        .route(
            "/{owner}/{name}/webhooks/{webhook_id}",
            axum::routing::delete(delete_webhook),
        )
}

#[derive(Serialize)]
struct WebhookResponse {
    id: String,
    url: String,
    events: Vec<String>,
    active: bool,
    created_at: String,
}

async fn list_webhooks(
    State(state): State<AppState>,
    Path((owner, name)): Path<(String, String)>,
    AuthUser(user): AuthUser,
) -> Result<Json<Vec<WebhookResponse>>, (StatusCode, String)> {
    let (repo, _) = resolve_admin_repo(&state, &owner, &name, &user).await?;

    let webhooks = delta_core::db::webhook::list_for_repo(&state.db, &repo.id.to_string())
        .await
        .map_err(|e| {
            tracing::error!("failed to list webhooks: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal server error".into(),
            )
        })?;

    Ok(Json(
        webhooks
            .into_iter()
            .map(|w| {
                let events: Vec<String> = serde_json::from_str(&w.events).unwrap_or_default();
                WebhookResponse {
                    id: w.id,
                    url: w.url,
                    events,
                    active: w.active,
                    created_at: w.created_at,
                }
            })
            .collect(),
    ))
}

#[derive(Deserialize)]
struct CreateWebhookRequest {
    url: String,
    secret: Option<String>,
    #[serde(default = "default_events")]
    events: Vec<String>,
}

fn default_events() -> Vec<String> {
    vec!["push".into()]
}

async fn create_webhook(
    State(state): State<AppState>,
    Path((owner, name)): Path<(String, String)>,
    AuthUser(user): AuthUser,
    Json(req): Json<CreateWebhookRequest>,
) -> Result<(StatusCode, Json<WebhookResponse>), (StatusCode, String)> {
    let (repo, _) = resolve_admin_repo(&state, &owner, &name, &user).await?;

    // Validate webhook URL: must be HTTP(S) and not target private networks
    if !req.url.starts_with("https://") && !req.url.starts_with("http://") {
        return Err((
            StatusCode::BAD_REQUEST,
            "webhook URL must use http or https".into(),
        ));
    }
    if crate::routes::git::is_private_url(&req.url) {
        return Err((
            StatusCode::BAD_REQUEST,
            "webhook URL must not target private networks".into(),
        ));
    }

    let events_json =
        serde_json::to_string(&req.events).map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    let id = delta_core::db::webhook::create(
        &state.db,
        &repo.id.to_string(),
        &req.url,
        req.secret.as_deref(),
        &events_json,
    )
    .await
    .map_err(|e| {
        tracing::error!("failed to create webhook: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal server error".into(),
        )
    })?;

    Ok((
        StatusCode::CREATED,
        Json(WebhookResponse {
            id,
            url: req.url,
            events: req.events,
            active: true,
            created_at: chrono::Utc::now().to_rfc3339(),
        }),
    ))
}

async fn delete_webhook(
    State(state): State<AppState>,
    Path((owner, name, webhook_id)): Path<(String, String, String)>,
    AuthUser(user): AuthUser,
) -> Result<StatusCode, (StatusCode, String)> {
    let (repo, _) = resolve_admin_repo(&state, &owner, &name, &user).await?;

    delta_core::db::webhook::delete(&state.db, &webhook_id, &repo.id.to_string())
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;

    Ok(StatusCode::NO_CONTENT)
}

/// Helper: resolve repo and verify admin access (owner or admin collaborator).
async fn resolve_admin_repo(
    state: &AppState,
    owner: &str,
    name: &str,
    user: &delta_core::models::user::User,
) -> Result<
    (
        delta_core::models::repo::Repository,
        delta_core::models::user::User,
    ),
    (StatusCode, String),
> {
    let owner_user = delta_core::db::user::get_by_username(&state.db, owner)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "user not found".into()))?;

    let owner_id = owner_user.id.to_string();
    let repo = delta_core::db::repo::get_by_owner_and_name(&state.db, &owner_id, name)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "repository not found".into()))?;

    require_role(state, &repo, &owner_user, user, CollaboratorRole::Admin).await?;

    Ok((repo, owner_user))
}
