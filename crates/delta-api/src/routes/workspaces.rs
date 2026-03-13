//! Remote agent workspace API routes.

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::get,
};
use base64::Engine;
use delta_core::db;
use delta_core::models::collaborator::CollaboratorRole;
use delta_core::models::workspace::WorkspaceStatus;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::extractors::AuthUser;
use crate::helpers::{require_role, resolve_repo_authed};
use crate::state::AppState;

/// Per-workspace lock map to prevent concurrent commit races.
pub type WorkspaceLocks = Arc<dashmap::DashMap<String, Arc<Mutex<()>>>>;

pub fn new_workspace_locks() -> WorkspaceLocks {
    Arc::new(dashmap::DashMap::new())
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/{owner}/{name}/workspaces",
            get(list_workspaces).post(create_workspace),
        )
        .route("/{owner}/{name}/workspaces/{ws_id}", get(get_workspace))
        .route(
            "/{owner}/{name}/workspaces/{ws_id}/files",
            axum::routing::post(write_files),
        )
        .route(
            "/{owner}/{name}/workspaces/{ws_id}/files/*path",
            get(read_file),
        )
        .route(
            "/{owner}/{name}/workspaces/{ws_id}/tree",
            get(list_tree),
        )
        .route(
            "/{owner}/{name}/workspaces/{ws_id}/pipelines",
            axum::routing::post(trigger_pipeline),
        )
        .route(
            "/{owner}/{name}/workspaces/{ws_id}/pull",
            axum::routing::post(create_pull),
        )
        .route(
            "/{owner}/{name}/workspaces/{ws_id}/close",
            axum::routing::post(close_workspace),
        )
        .route(
            "/{owner}/{name}/workspaces/{ws_id}/extend",
            axum::routing::post(extend_workspace),
        )
        .route(
            "/{owner}/{name}/workspaces/{ws_id}/diff",
            get(diff_workspace),
        )
}

// --- Request/Response types ---

#[derive(Deserialize)]
struct CreateWorkspaceRequest {
    name: String,
    base_branch: Option<String>,
    #[serde(default = "default_ttl")]
    ttl_hours: i64,
}
fn default_ttl() -> i64 {
    24
}

#[derive(Serialize)]
struct WorkspaceResponse {
    id: String,
    repo_id: String,
    creator_id: String,
    name: String,
    branch: String,
    base_branch: String,
    base_commit: String,
    head_commit: Option<String>,
    status: String,
    ttl_hours: i64,
    expires_at: String,
    created_at: String,
    updated_at: String,
}

impl From<delta_core::models::workspace::Workspace> for WorkspaceResponse {
    fn from(ws: delta_core::models::workspace::Workspace) -> Self {
        Self {
            id: ws.id.to_string(),
            repo_id: ws.repo_id,
            creator_id: ws.creator_id,
            name: ws.name,
            branch: ws.branch,
            base_branch: ws.base_branch,
            base_commit: ws.base_commit,
            head_commit: ws.head_commit,
            status: ws.status.as_str().to_string(),
            ttl_hours: ws.ttl_hours,
            expires_at: ws.expires_at.to_rfc3339(),
            created_at: ws.created_at.to_rfc3339(),
            updated_at: ws.updated_at.to_rfc3339(),
        }
    }
}

#[derive(Deserialize)]
struct WriteFilesRequest {
    message: String,
    files: Vec<FileEntry>,
}

#[derive(Deserialize)]
struct FileEntry {
    path: String,
    /// Base64-encoded content. Omit or null for deletion.
    content: Option<String>,
    /// Set to "delete" to remove a file.
    #[serde(default)]
    action: Option<String>,
}

#[derive(Serialize)]
struct WriteFilesResponse {
    commit_sha: String,
}

#[derive(Deserialize)]
struct ListWorkspacesQuery {
    status: Option<String>,
    #[serde(default = "default_ws_limit")]
    limit: i64,
}
fn default_ws_limit() -> i64 {
    50
}

#[derive(Deserialize)]
struct ExtendRequest {
    #[serde(default = "default_ttl")]
    hours: i64,
}

#[derive(Deserialize)]
struct CloseRequest {
    #[serde(default)]
    delete_branch: bool,
}

#[derive(Deserialize)]
struct CreatePullRequest {
    title: String,
    body: Option<String>,
    #[serde(default)]
    is_draft: bool,
}

#[derive(Deserialize)]
struct TriggerPipelineRequest {
    workflow_name: String,
}

// --- Handlers ---

async fn create_workspace(
    State(state): State<AppState>,
    Path((owner, name)): Path<(String, String)>,
    AuthUser(user): AuthUser,
    Json(req): Json<CreateWorkspaceRequest>,
) -> Result<(StatusCode, Json<WorkspaceResponse>), (StatusCode, String)> {
    let (repo, owner_user) = resolve_repo_authed(&state, &owner, &name, &user).await?;
    require_role(&state, &repo, &owner_user, &user, CollaboratorRole::Write).await?;

    // Validate name
    if req.name.is_empty() || req.name.len() > 100 {
        return Err((
            StatusCode::BAD_REQUEST,
            "workspace name must be 1-100 characters".into(),
        ));
    }
    if !req
        .name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "workspace name must be alphanumeric, hyphens, or underscores".into(),
        ));
    }

    let ttl = req.ttl_hours.clamp(1, 168); // 1h to 1 week
    let base_branch = req.base_branch.as_deref().unwrap_or(&repo.default_branch);
    let short_id = &uuid::Uuid::new_v4().to_string()[..8];
    let branch_name = format!("ws/{}/{}", short_id, req.name);

    let repo_path = state
        .repo_host
        .repo_path(&owner, &name)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    // Create the git branch
    let base_commit =
        delta_vcs::workspace::create_workspace_branch(&repo_path, &branch_name, base_branch)
            .await
            .map_err(|e| {
                tracing::error!("failed to create workspace branch: {}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "failed to create workspace branch".into(),
                )
            })?;

    // Create DB record
    let ws = db::workspace::create(
        &state.db,
        db::workspace::CreateWorkspaceParams {
            repo_id: &repo.id.to_string(),
            creator_id: &user.id.to_string(),
            name: &req.name,
            branch: &branch_name,
            base_branch,
            base_commit: &base_commit,
            ttl_hours: ttl,
        },
    )
    .await
    .map_err(|e| {
        tracing::error!("failed to create workspace: {}", e);
        (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
    })?;

    Ok((StatusCode::CREATED, Json(ws.into())))
}

async fn list_workspaces(
    State(state): State<AppState>,
    Path((owner, name)): Path<(String, String)>,
    AuthUser(user): AuthUser,
    Query(query): Query<ListWorkspacesQuery>,
) -> Result<Json<Vec<WorkspaceResponse>>, (StatusCode, String)> {
    let (repo, _) = resolve_repo_authed(&state, &owner, &name, &user).await?;
    let limit = query.limit.clamp(1, 200);
    let workspaces = db::workspace::list_for_repo(
        &state.db,
        &repo.id.to_string(),
        query.status.as_deref(),
        limit,
    )
    .await
    .map_err(|e| {
        tracing::error!("failed to list workspaces: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal server error".into(),
        )
    })?;
    Ok(Json(workspaces.into_iter().map(Into::into).collect()))
}

async fn get_workspace(
    State(state): State<AppState>,
    Path((owner, name, ws_id)): Path<(String, String, String)>,
    AuthUser(user): AuthUser,
) -> Result<Json<WorkspaceResponse>, (StatusCode, String)> {
    let (repo, _) = resolve_repo_authed(&state, &owner, &name, &user).await?;
    let ws = db::workspace::get_by_id(&state.db, &ws_id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    if ws.repo_id != repo.id.to_string() {
        return Err((StatusCode::NOT_FOUND, "workspace not found".into()));
    }
    Ok(Json(ws.into()))
}

async fn write_files(
    State(state): State<AppState>,
    Path((owner, name, ws_id)): Path<(String, String, String)>,
    AuthUser(user): AuthUser,
    Json(req): Json<WriteFilesRequest>,
) -> Result<Json<WriteFilesResponse>, (StatusCode, String)> {
    let (repo, owner_user) = resolve_repo_authed(&state, &owner, &name, &user).await?;
    require_role(&state, &repo, &owner_user, &user, CollaboratorRole::Write).await?;

    let ws = db::workspace::get_by_id(&state.db, &ws_id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    if ws.repo_id != repo.id.to_string() {
        return Err((StatusCode::NOT_FOUND, "workspace not found".into()));
    }
    if ws.status != WorkspaceStatus::Active {
        return Err((StatusCode::CONFLICT, "workspace is not active".into()));
    }

    if req.files.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "no files provided".into()));
    }

    // Decode files
    let mut file_writes = Vec::new();
    for entry in &req.files {
        let is_delete = entry.action.as_deref() == Some("delete");
        if is_delete {
            file_writes.push(delta_vcs::workspace::FileWrite {
                path: entry.path.clone(),
                content: None,
            });
        } else {
            let content = entry.content.as_deref().ok_or((
                StatusCode::BAD_REQUEST,
                format!("missing content for file '{}'", entry.path),
            ))?;
            let decoded = base64::engine::general_purpose::STANDARD
                .decode(content)
                .map_err(|e| {
                    (
                        StatusCode::BAD_REQUEST,
                        format!("invalid base64 for '{}': {}", entry.path, e),
                    )
                })?;
            file_writes.push(delta_vcs::workspace::FileWrite {
                path: entry.path.clone(),
                content: Some(decoded),
            });
        }
    }

    let repo_path = state
        .repo_host
        .repo_path(&owner, &name)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    // Acquire per-workspace lock
    let lock = state
        .workspace_locks
        .entry(ws_id.clone())
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone();
    let _guard = lock.lock().await;

    let author_name = user.display_name.as_deref().unwrap_or(&user.username);
    let commit_sha = delta_vcs::workspace::commit_workspace_files(
        &repo_path,
        &ws.branch,
        &file_writes,
        &req.message,
        author_name,
        &user.email,
    )
    .await
    .map_err(|e| {
        tracing::error!("failed to commit workspace files: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "failed to commit files".into(),
        )
    })?;

    // Update head commit in DB
    db::workspace::update_head_commit(&state.db, &ws_id, &commit_sha)
        .await
        .map_err(|e| {
            tracing::error!("failed to update head commit: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal server error".into(),
            )
        })?;

    Ok(Json(WriteFilesResponse { commit_sha }))
}

async fn read_file(
    State(state): State<AppState>,
    Path((owner, name, ws_id, path)): Path<(String, String, String, String)>,
    AuthUser(user): AuthUser,
) -> Result<String, (StatusCode, String)> {
    let (repo, _) = resolve_repo_authed(&state, &owner, &name, &user).await?;
    let ws = db::workspace::get_by_id(&state.db, &ws_id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    if ws.repo_id != repo.id.to_string() {
        return Err((StatusCode::NOT_FOUND, "workspace not found".into()));
    }

    let repo_path = state
        .repo_host
        .repo_path(&owner, &name)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    delta_vcs::browse::read_blob_text(&repo_path, &ws.branch, &path)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))
}

async fn list_tree(
    State(state): State<AppState>,
    Path((owner, name, ws_id)): Path<(String, String, String)>,
    AuthUser(user): AuthUser,
    Query(query): Query<TreeQuery>,
) -> Result<Json<Vec<delta_vcs::browse::TreeEntry>>, (StatusCode, String)> {
    let (repo, _) = resolve_repo_authed(&state, &owner, &name, &user).await?;
    let ws = db::workspace::get_by_id(&state.db, &ws_id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    if ws.repo_id != repo.id.to_string() {
        return Err((StatusCode::NOT_FOUND, "workspace not found".into()));
    }

    let repo_path = state
        .repo_host
        .repo_path(&owner, &name)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    let path = query.path.as_deref().unwrap_or("");
    let entries = delta_vcs::browse::list_tree(&repo_path, &ws.branch, path)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(entries))
}

#[derive(Deserialize)]
struct TreeQuery {
    path: Option<String>,
}

async fn trigger_pipeline(
    State(state): State<AppState>,
    Path((owner, name, ws_id)): Path<(String, String, String)>,
    AuthUser(user): AuthUser,
    Json(req): Json<TriggerPipelineRequest>,
) -> Result<(StatusCode, Json<db::pipeline::PipelineRun>), (StatusCode, String)> {
    let (repo, owner_user) = resolve_repo_authed(&state, &owner, &name, &user).await?;
    require_role(&state, &repo, &owner_user, &user, CollaboratorRole::Write).await?;

    let ws = db::workspace::get_by_id(&state.db, &ws_id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    if ws.repo_id != repo.id.to_string() {
        return Err((StatusCode::NOT_FOUND, "workspace not found".into()));
    }
    if ws.status != WorkspaceStatus::Active {
        return Err((StatusCode::CONFLICT, "workspace is not active".into()));
    }

    let commit_sha = ws
        .head_commit
        .as_deref()
        .unwrap_or(&ws.base_commit);

    let run = db::pipeline::create_pipeline(
        &state.db,
        &repo.id.to_string(),
        &req.workflow_name,
        "workspace",
        Some(&ws.branch),
        commit_sha,
    )
    .await
    .map_err(|e| {
        tracing::error!("failed to create pipeline: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal server error".into(),
        )
    })?;

    Ok((StatusCode::CREATED, Json(run)))
}

async fn create_pull(
    State(state): State<AppState>,
    Path((owner, name, ws_id)): Path<(String, String, String)>,
    AuthUser(user): AuthUser,
    Json(req): Json<CreatePullRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, String)> {
    let (repo, owner_user) = resolve_repo_authed(&state, &owner, &name, &user).await?;
    require_role(&state, &repo, &owner_user, &user, CollaboratorRole::Write).await?;

    let ws = db::workspace::get_by_id(&state.db, &ws_id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    if ws.repo_id != repo.id.to_string() {
        return Err((StatusCode::NOT_FOUND, "workspace not found".into()));
    }

    let pr = db::pull_request::create(
        &state.db,
        db::pull_request::CreatePrParams {
            repo_id: &repo.id.to_string(),
            author_id: &user.id.to_string(),
            title: &req.title,
            body: req.body.as_deref(),
            head_branch: &ws.branch,
            base_branch: &ws.base_branch,
            head_sha: ws.head_commit.as_deref(),
            is_draft: req.is_draft,
        },
    )
    .await
    .map_err(|e| {
        tracing::error!("failed to create PR: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "failed to create pull request".into(),
        )
    })?;

    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({
            "pull_request_id": pr.id.to_string(),
            "number": pr.number,
            "title": pr.title,
            "state": pr.state.as_str(),
            "head_branch": pr.head_branch,
            "base_branch": pr.base_branch,
        })),
    ))
}

async fn close_workspace(
    State(state): State<AppState>,
    Path((owner, name, ws_id)): Path<(String, String, String)>,
    AuthUser(user): AuthUser,
    Json(req): Json<CloseRequest>,
) -> Result<Json<WorkspaceResponse>, (StatusCode, String)> {
    let (repo, owner_user) = resolve_repo_authed(&state, &owner, &name, &user).await?;
    require_role(&state, &repo, &owner_user, &user, CollaboratorRole::Write).await?;

    let ws = db::workspace::get_by_id(&state.db, &ws_id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    if ws.repo_id != repo.id.to_string() {
        return Err((StatusCode::NOT_FOUND, "workspace not found".into()));
    }

    if req.delete_branch {
        let repo_path = state
            .repo_host
            .repo_path(&owner, &name)
            .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
        let _ = delta_vcs::workspace::delete_workspace_branch(&repo_path, &ws.branch).await;
    }

    let ws = db::workspace::update_status(&state.db, &ws_id, WorkspaceStatus::Closed)
        .await
        .map_err(|e| {
            tracing::error!("failed to close workspace: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal server error".into(),
            )
        })?;
    Ok(Json(ws.into()))
}

async fn extend_workspace(
    State(state): State<AppState>,
    Path((owner, name, ws_id)): Path<(String, String, String)>,
    AuthUser(user): AuthUser,
    Json(req): Json<ExtendRequest>,
) -> Result<Json<WorkspaceResponse>, (StatusCode, String)> {
    let (repo, owner_user) = resolve_repo_authed(&state, &owner, &name, &user).await?;
    require_role(&state, &repo, &owner_user, &user, CollaboratorRole::Write).await?;

    let ws = db::workspace::get_by_id(&state.db, &ws_id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    if ws.repo_id != repo.id.to_string() {
        return Err((StatusCode::NOT_FOUND, "workspace not found".into()));
    }
    if ws.status != WorkspaceStatus::Active {
        return Err((StatusCode::CONFLICT, "workspace is not active".into()));
    }

    let hours = req.hours.clamp(1, 168);
    let ws = db::workspace::extend_ttl(&state.db, &ws_id, hours)
        .await
        .map_err(|e| {
            tracing::error!("failed to extend workspace: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal server error".into(),
            )
        })?;
    Ok(Json(ws.into()))
}

async fn diff_workspace(
    State(state): State<AppState>,
    Path((owner, name, ws_id)): Path<(String, String, String)>,
    AuthUser(user): AuthUser,
) -> Result<String, (StatusCode, String)> {
    let (repo, _) = resolve_repo_authed(&state, &owner, &name, &user).await?;
    let ws = db::workspace::get_by_id(&state.db, &ws_id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    if ws.repo_id != repo.id.to_string() {
        return Err((StatusCode::NOT_FOUND, "workspace not found".into()));
    }

    let repo_path = state
        .repo_host
        .repo_path(&owner, &name)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    delta_vcs::diff::diff_refs(&repo_path, &ws.base_branch, &ws.branch)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

/// Cleanup task: expire workspaces past their TTL.
pub async fn cleanup_expired_workspaces(
    db: sqlx::SqlitePool,
    repo_host: Arc<delta_vcs::RepoHost>,
) {
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(300)).await;

        let expired = match db::workspace::list_expired(&db).await {
            Ok(ws) => ws,
            Err(e) => {
                tracing::error!("workspace cleanup: failed to list expired: {}", e);
                continue;
            }
        };

        for ws in expired {
            tracing::info!(
                workspace_id = %ws.id,
                branch = %ws.branch,
                "expiring workspace"
            );

            // Try to resolve repo for branch deletion
            if let Ok(repo) = db::repo::get_by_id(&db, &ws.repo_id).await
                && let Ok(repo_path) = repo_host.repo_path(&repo.owner, &repo.name)
            {
                let _ =
                    delta_vcs::workspace::delete_workspace_branch(&repo_path, &ws.branch).await;
                let _ = delta_vcs::workspace::prune_worktrees(&repo_path).await;
            }

            let _ = db::workspace::update_status(&db, &ws.id.to_string(), WorkspaceStatus::Expired)
                .await;
        }
    }
}
