use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::get,
};
use delta_core::db;
use delta_core::models::pull_request::*;
use serde::{Deserialize, Serialize};

use delta_core::models::collaborator::CollaboratorRole;

use crate::extractors::AuthUser;
use crate::helpers::{require_role, resolve_repo_authed};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/{owner}/{name}/pulls", get(list_pulls).post(create_pull))
        .route(
            "/{owner}/{name}/pulls/{number}",
            get(get_pull).put(update_pull),
        )
        .route(
            "/{owner}/{name}/pulls/{number}/merge",
            axum::routing::post(merge_pull),
        )
        .route(
            "/{owner}/{name}/pulls/{number}/close",
            axum::routing::post(close_pull),
        )
        .route(
            "/{owner}/{name}/pulls/{number}/reopen",
            axum::routing::post(reopen_pull),
        )
        .route("/{owner}/{name}/pulls/{number}/diff", get(get_diff))
        .route("/{owner}/{name}/pulls/{number}/commits", get(get_commits))
        .route(
            "/{owner}/{name}/pulls/{number}/comments",
            get(list_comments).post(create_comment),
        )
        .route(
            "/{owner}/{name}/pulls/{number}/reviews",
            get(list_reviews).post(submit_review),
        )
}

// --- Response types ---

#[derive(Serialize)]
struct PrResponse {
    id: String,
    number: i64,
    title: String,
    body: Option<String>,
    state: String,
    author: String,
    /// True if the PR was authored by an AI agent (provenance tracking).
    is_agent_authored: bool,
    head_branch: String,
    base_branch: String,
    head_sha: Option<String>,
    is_draft: bool,
    merged_by: Option<String>,
    merge_strategy: Option<String>,
    created_at: String,
    updated_at: String,
    merged_at: Option<String>,
    closed_at: Option<String>,
}

impl PrResponse {
    async fn from_pr(pr: PullRequest, pool: &sqlx::SqlitePool) -> Self {
        let author_user = db::user::get_by_id(pool, &pr.author_id.to_string()).await;
        let author = author_user
            .as_ref()
            .map(|u| u.username.clone())
            .unwrap_or_else(|_| pr.author_id.to_string());
        let is_agent_authored = author_user.map(|u| u.is_agent).unwrap_or(false);

        let merged_by = if let Some(id) = &pr.merged_by {
            db::user::get_by_id(pool, &id.to_string())
                .await
                .map(|u| Some(u.username))
                .unwrap_or(None)
        } else {
            None
        };

        let state_str = pr.state.as_str().to_string();

        let strategy_str = pr.merge_strategy.map(|s| s.as_str().to_string());

        PrResponse {
            id: pr.id.to_string(),
            number: pr.number,
            title: pr.title,
            body: pr.body,
            state: state_str,
            author,
            is_agent_authored,
            head_branch: pr.head_branch,
            base_branch: pr.base_branch,
            head_sha: pr.head_sha,
            is_draft: pr.is_draft,
            merged_by,
            merge_strategy: strategy_str,
            created_at: pr.created_at.to_rfc3339(),
            updated_at: pr.updated_at.to_rfc3339(),
            merged_at: pr.merged_at.map(|d| d.to_rfc3339()),
            closed_at: pr.closed_at.map(|d| d.to_rfc3339()),
        }
    }
}

#[derive(Serialize)]
struct CommentResponse {
    id: String,
    author: String,
    body: String,
    file_path: Option<String>,
    line: Option<i64>,
    side: Option<String>,
    created_at: String,
    updated_at: String,
}

#[derive(Serialize)]
struct ReviewResponse {
    id: String,
    reviewer: String,
    state: String,
    body: Option<String>,
    created_at: String,
}

// --- Handlers ---

#[derive(Deserialize)]
struct ListPullsQuery {
    state: Option<String>,
}

async fn list_pulls(
    State(state): State<AppState>,
    Path((owner, name)): Path<(String, String)>,
    AuthUser(user): AuthUser,
    Query(query): Query<ListPullsQuery>,
) -> Result<Json<Vec<PrResponse>>, (StatusCode, String)> {
    let (repo, _) = resolve_repo_authed(&state, &owner, &name, &user).await?;
    let repo_id = repo.id.to_string();

    let prs = db::pull_request::list_for_repo(&state.db, &repo_id, query.state.as_deref())
        .await
        .map_err(|e| {
            tracing::error!("failed to list pull requests: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal server error".into(),
            )
        })?;

    let mut responses = Vec::new();
    for pr in prs {
        responses.push(PrResponse::from_pr(pr, &state.db).await);
    }
    Ok(Json(responses))
}

#[derive(Deserialize)]
struct CreatePrRequest {
    title: String,
    body: Option<String>,
    head_branch: String,
    base_branch: String,
    #[serde(default)]
    is_draft: bool,
}

async fn create_pull(
    State(state): State<AppState>,
    Path((owner, name)): Path<(String, String)>,
    AuthUser(user): AuthUser,
    Json(req): Json<CreatePrRequest>,
) -> Result<(StatusCode, Json<PrResponse>), (StatusCode, String)> {
    // Validate input lengths
    if req.title.is_empty() || req.title.len() > 256 {
        return Err((
            StatusCode::BAD_REQUEST,
            "title must be 1-256 characters".into(),
        ));
    }
    if let Some(body) = &req.body
        && body.len() > 65536
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "body must be at most 65536 characters".into(),
        ));
    }
    if req.head_branch.is_empty()
        || req.head_branch.len() > 256
        || !is_valid_branch_name(&req.head_branch)
    {
        return Err((StatusCode::BAD_REQUEST, "invalid head branch name".into()));
    }
    if req.base_branch.is_empty()
        || req.base_branch.len() > 256
        || !is_valid_branch_name(&req.base_branch)
    {
        return Err((StatusCode::BAD_REQUEST, "invalid base branch name".into()));
    }

    let (repo, _) = resolve_repo_authed(&state, &owner, &name, &user).await?;
    let repo_id = repo.id.to_string();
    let user_id = user.id.to_string();

    // Get head SHA if possible
    let repo_path = state
        .repo_host
        .repo_path(&owner, &name)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    let head_sha = delta_vcs::refs::list_branches(&repo_path)
        .ok()
        .and_then(|branches| {
            branches
                .iter()
                .find(|b| b.name == req.head_branch)
                .map(|b| b.commit_id.clone())
        });

    let pr = db::pull_request::create(
        &state.db,
        db::pull_request::CreatePrParams {
            repo_id: &repo_id,
            author_id: &user_id,
            title: &req.title,
            body: req.body.as_deref(),
            head_branch: &req.head_branch,
            base_branch: &req.base_branch,
            head_sha: head_sha.as_deref(),
            is_draft: req.is_draft,
        },
    )
    .await
    .map_err(|e| {
        tracing::error!("failed to create pull request: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal server error".into(),
        )
    })?;

    Ok((
        StatusCode::CREATED,
        Json(PrResponse::from_pr(pr, &state.db).await),
    ))
}

async fn get_pull(
    State(state): State<AppState>,
    Path((owner, name, number)): Path<(String, String, i64)>,
    AuthUser(user): AuthUser,
) -> Result<Json<PrResponse>, (StatusCode, String)> {
    let (repo, _) = resolve_repo_authed(&state, &owner, &name, &user).await?;
    let repo_id = repo.id.to_string();

    let pr = db::pull_request::get_by_number(&state.db, &repo_id, number)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;

    Ok(Json(PrResponse::from_pr(pr, &state.db).await))
}

#[derive(Deserialize)]
struct UpdatePrRequest {
    title: Option<String>,
    body: Option<String>,
}

async fn update_pull(
    State(state): State<AppState>,
    Path((owner, name, number)): Path<(String, String, i64)>,
    AuthUser(user): AuthUser,
    Json(req): Json<UpdatePrRequest>,
) -> Result<Json<PrResponse>, (StatusCode, String)> {
    let (repo, _) = resolve_repo_authed(&state, &owner, &name, &user).await?;
    let repo_id = repo.id.to_string();

    let pr = db::pull_request::get_by_number(&state.db, &repo_id, number)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;

    if pr.state != PrState::Open {
        return Err((
            StatusCode::CONFLICT,
            "can only update open pull requests".into(),
        ));
    }

    // Validate input lengths
    if let Some(ref title) = req.title
        && (title.is_empty() || title.len() > 256)
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "title must be 1-256 characters".into(),
        ));
    }
    if let Some(ref body) = req.body
        && body.len() > 65536
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "body must be at most 65536 characters".into(),
        ));
    }

    // Author, owner, or write+ collaborators can update
    if pr.author_id != user.id {
        let owner_user = db::user::get_by_id(&state.db, &repo.owner)
            .await
            .map_err(|_| (StatusCode::NOT_FOUND, "owner not found".into()))?;
        require_role(&state, &repo, &owner_user, &user, CollaboratorRole::Write).await?;
    }

    let updated = db::pull_request::update_title_body(
        &state.db,
        &pr.id.to_string(),
        req.title.as_deref(),
        req.body.as_deref(),
    )
    .await
    .map_err(|e| {
        tracing::error!("failed to update pull request: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal server error".into(),
        )
    })?;

    Ok(Json(PrResponse::from_pr(updated, &state.db).await))
}

#[derive(Deserialize)]
struct MergePrRequest {
    #[serde(default = "default_merge_strategy")]
    strategy: String,
    message: Option<String>,
}

fn default_merge_strategy() -> String {
    "merge".to_string()
}

async fn merge_pull(
    State(state): State<AppState>,
    Path((owner, name, number)): Path<(String, String, i64)>,
    AuthUser(user): AuthUser,
    Json(req): Json<MergePrRequest>,
) -> Result<Json<PrResponse>, (StatusCode, String)> {
    let (repo, owner_user) = resolve_repo_authed(&state, &owner, &name, &user).await?;
    let repo_id = repo.id.to_string();

    require_role(&state, &repo, &owner_user, &user, CollaboratorRole::Write).await?;

    let pr = db::pull_request::get_by_number(&state.db, &repo_id, number)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;

    if pr.state != PrState::Open {
        return Err((StatusCode::CONFLICT, "pull request is not open".into()));
    }

    // Check branch protection
    if let Ok(Some(protection)) =
        db::branch_protection::find_matching(&state.db, &repo_id, &pr.base_branch).await
    {
        // Check required approvals
        if protection.required_approvals > 0 {
            let approvals = db::pull_request::count_approvals(&state.db, &pr.id.to_string())
                .await
                .unwrap_or(0);
            if approvals < protection.required_approvals {
                return Err((
                    StatusCode::CONFLICT,
                    format!(
                        "requires {} approval(s), has {}",
                        protection.required_approvals, approvals
                    ),
                ));
            }
        }

        // Check status checks
        if protection.require_status_checks {
            match &pr.head_sha {
                Some(sha) => {
                    let passed = db::status_check::all_passed(&state.db, &repo_id, sha)
                        .await
                        .unwrap_or(false);
                    if !passed {
                        return Err((
                            StatusCode::CONFLICT,
                            "status checks have not all passed".into(),
                        ));
                    }
                }
                None => {
                    return Err((
                        StatusCode::CONFLICT,
                        "cannot verify status checks: head SHA is unknown".into(),
                    ));
                }
            }
        }
    }

    // Execute the merge
    let repo_path = state
        .repo_host
        .repo_path(&owner, &name)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    let merge_mode = match req.strategy.as_str() {
        "squash" => delta_vcs::merge::MergeMode::Squash,
        "rebase" => delta_vcs::merge::MergeMode::Rebase,
        _ => delta_vcs::merge::MergeMode::Merge,
    };

    let merge_message = req.message.unwrap_or_else(|| {
        format!(
            "Merge pull request #{} from {}\n\n{}",
            pr.number, pr.head_branch, pr.title
        )
    });
    if merge_message.len() > 4096 {
        return Err((
            StatusCode::BAD_REQUEST,
            "merge message too long (max 4096 characters)".into(),
        ));
    }

    let _merge_sha = delta_vcs::merge::execute_merge(
        &repo_path,
        &pr.base_branch,
        &pr.head_branch,
        merge_mode,
        &merge_message,
        &user.username,
        &user.email,
    )
    .await
    .map_err(|e| (StatusCode::CONFLICT, format!("merge failed: {}", e)))?;

    // Mark as merged in DB
    let merged = db::pull_request::mark_merged(
        &state.db,
        &pr.id.to_string(),
        &user.id.to_string(),
        &req.strategy,
    )
    .await
    .map_err(|e| {
        tracing::error!("failed to mark PR as merged: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal server error".into(),
        )
    })?;

    Ok(Json(PrResponse::from_pr(merged, &state.db).await))
}

async fn close_pull(
    State(state): State<AppState>,
    Path((owner, name, number)): Path<(String, String, i64)>,
    AuthUser(user): AuthUser,
) -> Result<Json<PrResponse>, (StatusCode, String)> {
    let (repo, owner_user) = resolve_repo_authed(&state, &owner, &name, &user).await?;
    let repo_id = repo.id.to_string();

    let pr = db::pull_request::get_by_number(&state.db, &repo_id, number)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;

    if pr.author_id != user.id {
        require_role(&state, &repo, &owner_user, &user, CollaboratorRole::Write).await?;
    }

    let closed = db::pull_request::close(&state.db, &pr.id.to_string())
        .await
        .map_err(|e| {
            tracing::error!("failed to close pull request: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal server error".into(),
            )
        })?;

    Ok(Json(PrResponse::from_pr(closed, &state.db).await))
}

async fn reopen_pull(
    State(state): State<AppState>,
    Path((owner, name, number)): Path<(String, String, i64)>,
    AuthUser(user): AuthUser,
) -> Result<Json<PrResponse>, (StatusCode, String)> {
    let (repo, owner_user) = resolve_repo_authed(&state, &owner, &name, &user).await?;
    let repo_id = repo.id.to_string();

    let pr = db::pull_request::get_by_number(&state.db, &repo_id, number)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;

    if pr.author_id != user.id {
        require_role(&state, &repo, &owner_user, &user, CollaboratorRole::Write).await?;
    }

    let reopened = db::pull_request::reopen(&state.db, &pr.id.to_string())
        .await
        .map_err(|e| {
            tracing::error!("failed to reopen pull request: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal server error".into(),
            )
        })?;

    Ok(Json(PrResponse::from_pr(reopened, &state.db).await))
}

async fn get_diff(
    State(state): State<AppState>,
    Path((owner, name, number)): Path<(String, String, i64)>,
    AuthUser(user): AuthUser,
) -> Result<
    (
        StatusCode,
        [(axum::http::HeaderName, &'static str); 1],
        String,
    ),
    (StatusCode, String),
> {
    let (repo, _) = resolve_repo_authed(&state, &owner, &name, &user).await?;
    let repo_id = repo.id.to_string();

    let pr = db::pull_request::get_by_number(&state.db, &repo_id, number)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;

    let repo_path = state
        .repo_host
        .repo_path(&owner, &name)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    let diff = delta_vcs::diff::diff_refs(&repo_path, &pr.base_branch, &pr.head_branch)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok((
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "text/plain")],
        diff,
    ))
}

async fn get_commits(
    State(state): State<AppState>,
    Path((owner, name, number)): Path<(String, String, i64)>,
    AuthUser(user): AuthUser,
) -> Result<Json<Vec<delta_vcs::diff::CommitInfo>>, (StatusCode, String)> {
    let (repo, _) = resolve_repo_authed(&state, &owner, &name, &user).await?;
    let repo_id = repo.id.to_string();

    let pr = db::pull_request::get_by_number(&state.db, &repo_id, number)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;

    let repo_path = state
        .repo_host
        .repo_path(&owner, &name)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    let commits = delta_vcs::diff::list_commits(&repo_path, &pr.base_branch, &pr.head_branch)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(commits))
}

// --- Comments ---

async fn list_comments(
    State(state): State<AppState>,
    Path((owner, name, number)): Path<(String, String, i64)>,
    AuthUser(user): AuthUser,
) -> Result<Json<Vec<CommentResponse>>, (StatusCode, String)> {
    let (repo, _) = resolve_repo_authed(&state, &owner, &name, &user).await?;
    let repo_id = repo.id.to_string();

    let pr = db::pull_request::get_by_number(&state.db, &repo_id, number)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;

    let comments = db::pull_request::list_comments(&state.db, &pr.id.to_string())
        .await
        .map_err(|e| {
            tracing::error!("failed to list comments: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal server error".into(),
            )
        })?;

    let mut responses = Vec::new();
    for c in comments {
        let author = db::user::get_by_id(&state.db, &c.author_id.to_string())
            .await
            .map(|u| u.username)
            .unwrap_or_else(|_| c.author_id.to_string());
        responses.push(CommentResponse {
            id: c.id.to_string(),
            author,
            body: c.body,
            file_path: c.file_path,
            line: c.line,
            side: c.side,
            created_at: c.created_at.to_rfc3339(),
            updated_at: c.updated_at.to_rfc3339(),
        });
    }
    Ok(Json(responses))
}

#[derive(Deserialize)]
struct CreateCommentRequest {
    body: String,
    file_path: Option<String>,
    line: Option<i64>,
    side: Option<String>,
}

async fn create_comment(
    State(state): State<AppState>,
    Path((owner, name, number)): Path<(String, String, i64)>,
    AuthUser(user): AuthUser,
    Json(req): Json<CreateCommentRequest>,
) -> Result<(StatusCode, Json<CommentResponse>), (StatusCode, String)> {
    if req.body.is_empty() || req.body.len() > 65536 {
        return Err((
            StatusCode::BAD_REQUEST,
            "comment body must be 1-65536 characters".into(),
        ));
    }

    let (repo, _) = resolve_repo_authed(&state, &owner, &name, &user).await?;
    let repo_id = repo.id.to_string();

    let pr = db::pull_request::get_by_number(&state.db, &repo_id, number)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;

    let comment = db::pull_request::add_comment(
        &state.db,
        &pr.id.to_string(),
        &user.id.to_string(),
        &req.body,
        req.file_path.as_deref(),
        req.line,
        req.side.as_deref(),
    )
    .await
    .map_err(|e| {
        tracing::error!("failed to add comment: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal server error".into(),
        )
    })?;

    Ok((
        StatusCode::CREATED,
        Json(CommentResponse {
            id: comment.id.to_string(),
            author: user.username,
            body: comment.body,
            file_path: comment.file_path,
            line: comment.line,
            side: comment.side,
            created_at: comment.created_at.to_rfc3339(),
            updated_at: comment.updated_at.to_rfc3339(),
        }),
    ))
}

// --- Reviews ---

async fn list_reviews(
    State(state): State<AppState>,
    Path((owner, name, number)): Path<(String, String, i64)>,
    AuthUser(user): AuthUser,
) -> Result<Json<Vec<ReviewResponse>>, (StatusCode, String)> {
    let (repo, _) = resolve_repo_authed(&state, &owner, &name, &user).await?;
    let repo_id = repo.id.to_string();

    let pr = db::pull_request::get_by_number(&state.db, &repo_id, number)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;

    let reviews = db::pull_request::list_reviews(&state.db, &pr.id.to_string())
        .await
        .map_err(|e| {
            tracing::error!("failed to list reviews: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal server error".into(),
            )
        })?;

    let mut responses = Vec::new();
    for r in reviews {
        let reviewer = db::user::get_by_id(&state.db, &r.reviewer_id.to_string())
            .await
            .map(|u| u.username)
            .unwrap_or_else(|_| r.reviewer_id.to_string());
        responses.push(ReviewResponse {
            id: r.id.to_string(),
            reviewer,
            state: r.state.as_str().to_string(),
            body: r.body,
            created_at: r.created_at.to_rfc3339(),
        });
    }
    Ok(Json(responses))
}

#[derive(Deserialize)]
struct SubmitReviewRequest {
    state: String,
    body: Option<String>,
}

async fn submit_review(
    State(state): State<AppState>,
    Path((owner, name, number)): Path<(String, String, i64)>,
    AuthUser(user): AuthUser,
    Json(req): Json<SubmitReviewRequest>,
) -> Result<(StatusCode, Json<ReviewResponse>), (StatusCode, String)> {
    let (repo, _) = resolve_repo_authed(&state, &owner, &name, &user).await?;
    let repo_id = repo.id.to_string();

    let pr = db::pull_request::get_by_number(&state.db, &repo_id, number)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;

    // Can't review your own PR
    if pr.author_id == user.id {
        return Err((
            StatusCode::CONFLICT,
            "cannot review your own pull request".into(),
        ));
    }

    let review_state = match req.state.as_str() {
        "approved" => ReviewState::Approved,
        "changes_requested" => ReviewState::ChangesRequested,
        _ => ReviewState::Commented,
    };

    let review = db::pull_request::submit_review(
        &state.db,
        &pr.id.to_string(),
        &user.id.to_string(),
        review_state,
        req.body.as_deref(),
    )
    .await
    .map_err(|e| {
        tracing::error!("failed to submit review: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal server error".into(),
        )
    })?;

    Ok((
        StatusCode::CREATED,
        Json(ReviewResponse {
            id: review.id.to_string(),
            reviewer: user.username,
            state: review.state.as_str().to_string(),
            body: review.body,
            created_at: review.created_at.to_rfc3339(),
        }),
    ))
}

/// Validate branch name: no null bytes, control chars, spaces, `..`, `~`, `^`, `:`, `\`, `?`, `*`, `[`.
fn is_valid_branch_name(name: &str) -> bool {
    !name.contains('\0')
        && !name.contains("..")
        && !name.ends_with('.')
        && !name.ends_with('/')
        && !name.starts_with('-')
        && !name
            .chars()
            .any(|c| c.is_control() || matches!(c, ' ' | '~' | '^' | ':' | '\\' | '?' | '*' | '['))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_branch_names() {
        assert!(is_valid_branch_name("main"));
        assert!(is_valid_branch_name("feature/my-feature"));
        assert!(is_valid_branch_name("release/v1.0.0"));
        assert!(is_valid_branch_name("fix/bug-123"));
        assert!(is_valid_branch_name("a"));
        assert!(is_valid_branch_name("refs/heads/main"));
    }

    #[test]
    fn test_invalid_branch_null_byte() {
        assert!(!is_valid_branch_name("main\0"));
        assert!(!is_valid_branch_name("fea\0ture"));
    }

    #[test]
    fn test_invalid_branch_double_dot() {
        assert!(!is_valid_branch_name("main..dev"));
        assert!(!is_valid_branch_name("..hidden"));
    }

    #[test]
    fn test_invalid_branch_trailing_dot() {
        assert!(!is_valid_branch_name("main."));
        assert!(!is_valid_branch_name("feature."));
    }

    #[test]
    fn test_invalid_branch_trailing_slash() {
        assert!(!is_valid_branch_name("feature/"));
        assert!(!is_valid_branch_name("dir/"));
    }

    #[test]
    fn test_invalid_branch_leading_hyphen() {
        assert!(!is_valid_branch_name("-feature"));
        assert!(!is_valid_branch_name("-"));
    }

    #[test]
    fn test_invalid_branch_control_chars() {
        assert!(!is_valid_branch_name("main\x01"));
        assert!(!is_valid_branch_name("feat\x7f"));
        assert!(!is_valid_branch_name("\t"));
    }

    #[test]
    fn test_invalid_branch_special_chars() {
        assert!(!is_valid_branch_name("main dev")); // space
        assert!(!is_valid_branch_name("feat~1")); // tilde
        assert!(!is_valid_branch_name("feat^2")); // caret
        assert!(!is_valid_branch_name("a:b")); // colon
        assert!(!is_valid_branch_name("a\\b")); // backslash
        assert!(!is_valid_branch_name("feat?")); // question mark
        assert!(!is_valid_branch_name("feat*")); // asterisk
        assert!(!is_valid_branch_name("feat[0]")); // bracket
    }
}
