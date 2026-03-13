//! AI-powered API endpoints.
//!
//! Provides structured responses optimized for LLM consumption,
//! AI code review, PR generation, and natural language queries.

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
};
use delta_core::{ai::AiClient, db};
use serde::{Deserialize, Serialize};

use crate::extractors::AuthUser;
use crate::helpers;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        // Structured API responses (LLM-friendly)
        .route("/{owner}/{name}/structured", get(structured_repo))
        .route("/{owner}/{name}/structured/tree", get(structured_tree))
        .route("/{owner}/{name}/structured/pulls", get(structured_pulls))
        // AI-powered features
        .route("/{owner}/{name}/ai/review/{number}", post(ai_review_pr))
        .route("/{owner}/{name}/ai/describe-pr", post(ai_describe_pr))
        .route(
            "/{owner}/{name}/ai/summarize-commit/{sha}",
            post(ai_summarize_commit),
        )
        .route("/{owner}/{name}/ai/query", post(ai_query_repo))
        // Code search
        .route("/{owner}/{name}/search", get(search_code))
        .route("/{owner}/{name}/search/index", post(index_repo))
}

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct StructuredRepoResponse {
    repository: RepoOverview,
    branches: Vec<BranchOverview>,
    recent_commits: Vec<CommitOverview>,
    open_pull_requests: Vec<PrOverview>,
    file_tree: Vec<TreeEntryOverview>,
}

#[derive(Serialize)]
struct RepoOverview {
    owner: String,
    name: String,
    description: Option<String>,
    visibility: String,
    default_branch: String,
}

#[derive(Serialize)]
struct BranchOverview {
    name: String,
    commit_id: String,
    is_default: bool,
}

#[derive(Serialize)]
struct CommitOverview {
    sha: String,
    author: String,
    message: String,
    date: String,
}

#[derive(Serialize)]
struct PrOverview {
    number: i64,
    title: String,
    state: String,
    author: String,
    head_branch: String,
    base_branch: String,
    created_at: String,
}

#[derive(Serialize)]
struct TreeEntryOverview {
    name: String,
    path: String,
    kind: String,
    mode: String,
}

#[derive(Serialize)]
struct StructuredPullResponse {
    number: i64,
    title: String,
    body: Option<String>,
    state: String,
    author: String,
    head_branch: String,
    base_branch: String,
    is_draft: bool,
    review_status: ReviewStatusOverview,
    created_at: String,
    updated_at: String,
}

#[derive(Serialize)]
struct ReviewStatusOverview {
    approvals: i64,
    changes_requested: i64,
    comments: i64,
}

#[derive(Serialize)]
struct AiReviewResponse {
    review: delta_core::ai::ReviewSummary,
    posted: bool,
}

#[derive(Deserialize)]
struct DescribePrRequest {
    head_branch: String,
    base_branch: String,
}

#[derive(Serialize)]
struct DescribePrResponse {
    title: String,
    body: String,
}

#[derive(Deserialize)]
struct QueryRequest {
    question: String,
}

#[derive(Serialize)]
struct QueryResponse {
    answer: String,
    relevant_files: Vec<String>,
}

#[derive(Deserialize)]
struct SearchQuery {
    q: String,
    #[serde(default = "default_search_limit")]
    limit: u32,
}

fn default_search_limit() -> u32 {
    20
}

#[derive(Deserialize)]
struct TreeQuery {
    #[serde(default)]
    path: String,
    #[serde(default = "default_rev")]
    rev: String,
}

fn default_rev() -> String {
    "HEAD".to_string()
}

#[derive(Deserialize)]
struct PullsQuery {
    #[serde(default)]
    state: Option<String>,
}

#[derive(Deserialize)]
struct ReviewQuery {
    #[serde(default)]
    post: bool,
}

// ---------------------------------------------------------------------------
// Structured endpoints (read-only, repo:read scope)
// ---------------------------------------------------------------------------

async fn structured_repo(
    State(state): State<AppState>,
    Path((owner, name)): Path<(String, String)>,
    AuthUser(user): AuthUser,
) -> Result<Json<StructuredRepoResponse>, (StatusCode, String)> {
    let (repo, owner_user) = helpers::resolve_repo_authed(&state, &owner, &name, &user).await?;

    // Branches
    let repo_path = state
        .repo_host
        .repo_path(&owner, &name)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    let branches: Vec<BranchOverview> = delta_vcs::refs::list_branches(&repo_path)
        .unwrap_or_default()
        .into_iter()
        .map(|b| BranchOverview {
            name: b.name,
            commit_id: b.commit_id,
            is_default: b.is_default,
        })
        .collect();

    // Recent commits
    let recent_commits: Vec<CommitOverview> = delta_vcs::browse::log(&repo_path, "HEAD", None, 10)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|c| CommitOverview {
            sha: c.sha,
            author: c.author_name,
            message: c.message,
            date: c.date,
        })
        .collect();

    // Open PRs
    let repo_id = repo.id.to_string();
    let prs = db::pull_request::list_for_repo(&state.db, &repo_id, Some("open"))
        .await
        .unwrap_or_default();

    let mut open_pull_requests = Vec::new();
    for pr in prs {
        let author = db::user::get_by_id(&state.db, &pr.author_id.to_string())
            .await
            .map(|u| u.username)
            .unwrap_or_else(|_| pr.author_id.to_string());
        open_pull_requests.push(PrOverview {
            number: pr.number,
            title: pr.title,
            state: pr.state.as_str().to_string(),
            author,
            head_branch: pr.head_branch,
            base_branch: pr.base_branch,
            created_at: pr.created_at.to_rfc3339(),
        });
    }

    // Top-level file tree
    let file_tree: Vec<TreeEntryOverview> = delta_vcs::browse::list_tree(&repo_path, "HEAD", "")
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|e| TreeEntryOverview {
            name: e.name,
            path: e.path,
            kind: e.kind,
            mode: e.mode,
        })
        .collect();

    Ok(Json(StructuredRepoResponse {
        repository: RepoOverview {
            owner: owner_user.username,
            name: repo.name,
            description: repo.description,
            visibility: repo.visibility.as_str().to_string(),
            default_branch: repo.default_branch,
        },
        branches,
        recent_commits,
        open_pull_requests,
        file_tree,
    }))
}

async fn structured_tree(
    State(state): State<AppState>,
    Path((owner, name)): Path<(String, String)>,
    AuthUser(user): AuthUser,
    Query(query): Query<TreeQuery>,
) -> Result<Json<Vec<TreeEntryOverview>>, (StatusCode, String)> {
    let (_repo, _owner_user) = helpers::resolve_repo_authed(&state, &owner, &name, &user).await?;

    let repo_path = state
        .repo_host
        .repo_path(&owner, &name)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    let entries = delta_vcs::browse::list_tree(&repo_path, &query.rev, &query.path)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let result: Vec<TreeEntryOverview> = entries
        .into_iter()
        .map(|e| TreeEntryOverview {
            name: e.name,
            path: e.path,
            kind: e.kind,
            mode: e.mode,
        })
        .collect();

    Ok(Json(result))
}

async fn structured_pulls(
    State(state): State<AppState>,
    Path((owner, name)): Path<(String, String)>,
    AuthUser(user): AuthUser,
    Query(query): Query<PullsQuery>,
) -> Result<Json<Vec<StructuredPullResponse>>, (StatusCode, String)> {
    let (repo, _owner_user) = helpers::resolve_repo_authed(&state, &owner, &name, &user).await?;
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
        let author = db::user::get_by_id(&state.db, &pr.author_id.to_string())
            .await
            .map(|u| u.username)
            .unwrap_or_else(|_| pr.author_id.to_string());

        let reviews = db::pull_request::list_reviews(&state.db, &pr.id.to_string())
            .await
            .unwrap_or_default();

        let mut approvals = 0i64;
        let mut changes_requested = 0i64;
        let mut comments = 0i64;
        for r in &reviews {
            match r.state.as_str() {
                "approved" => approvals += 1,
                "changes_requested" => changes_requested += 1,
                _ => comments += 1,
            }
        }

        responses.push(StructuredPullResponse {
            number: pr.number,
            title: pr.title,
            body: pr.body,
            state: pr.state.as_str().to_string(),
            author,
            head_branch: pr.head_branch,
            base_branch: pr.base_branch,
            is_draft: pr.is_draft,
            review_status: ReviewStatusOverview {
                approvals,
                changes_requested,
                comments,
            },
            created_at: pr.created_at.to_rfc3339(),
            updated_at: pr.updated_at.to_rfc3339(),
        });
    }

    Ok(Json(responses))
}

// ---------------------------------------------------------------------------
// AI-powered endpoints (require ai scope and AI enabled)
// ---------------------------------------------------------------------------

fn require_ai(state: &AppState) -> Result<AiClient, (StatusCode, String)> {
    if !AiClient::is_available(&state.config.ai) {
        return Err((
            StatusCode::NOT_IMPLEMENTED,
            "AI features are not enabled. Configure the [ai] section in your Delta config with \
             enabled=true, provider, and api_key to use AI-powered features."
                .into(),
        ));
    }
    AiClient::new(&state.config.ai).map_err(|e| {
        tracing::error!("failed to create AI client: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "failed to initialize AI client".into(),
        )
    })
}

async fn ai_review_pr(
    State(state): State<AppState>,
    Path((owner, name, number)): Path<(String, String, i64)>,
    AuthUser(user): AuthUser,
    Query(review_query): Query<ReviewQuery>,
) -> Result<Json<AiReviewResponse>, (StatusCode, String)> {
    let ai = require_ai(&state)?;
    let (repo, _owner_user) = helpers::resolve_repo_authed(&state, &owner, &name, &user).await?;
    let repo_id = repo.id.to_string();

    let pr = db::pull_request::get_by_number(&state.db, &repo_id, number)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;

    // Get the diff
    let repo_path = state
        .repo_host
        .repo_path(&owner, &name)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    let diff = delta_vcs::diff::diff_refs(&repo_path, &pr.base_branch, &pr.head_branch)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let context = format!(
        "Repository: {}/{}\nPR #{}: {}\nBranch: {} -> {}",
        owner, name, pr.number, pr.title, pr.head_branch, pr.base_branch
    );

    let review = ai.review_diff(&diff, &context).await.map_err(|e| {
        tracing::error!("AI review failed: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("AI review failed: {}", e),
        )
    })?;

    // Optionally post as a review comment
    let posted = if review_query.post {
        let body = format!(
            "## AI Code Review\n\n**Risk Level:** {}\n\n{}\n\n### Issues\n{}\n\n### Suggestions\n{}",
            review.risk_level,
            review.summary,
            review
                .issues
                .iter()
                .map(|i| format!("- **{}** ({}): {}", i.file, i.severity, i.message))
                .collect::<Vec<_>>()
                .join("\n"),
            review
                .suggestions
                .iter()
                .map(|s| format!("- **{}**: {}", s.file, s.explanation))
                .collect::<Vec<_>>()
                .join("\n"),
        );

        let _ = db::pull_request::add_comment(
            &state.db,
            &pr.id.to_string(),
            &user.id.to_string(),
            &body,
            None,
            None,
            None,
        )
        .await;
        true
    } else {
        false
    };

    Ok(Json(AiReviewResponse { review, posted }))
}

async fn ai_describe_pr(
    State(state): State<AppState>,
    Path((owner, name)): Path<(String, String)>,
    AuthUser(user): AuthUser,
    Json(req): Json<DescribePrRequest>,
) -> Result<Json<DescribePrResponse>, (StatusCode, String)> {
    let ai = require_ai(&state)?;
    let (_repo, _owner_user) = helpers::resolve_repo_authed(&state, &owner, &name, &user).await?;

    let repo_path = state
        .repo_host
        .repo_path(&owner, &name)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    let diff = delta_vcs::diff::diff_refs(&repo_path, &req.base_branch, &req.head_branch)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let commits = delta_vcs::diff::list_commits(&repo_path, &req.base_branch, &req.head_branch)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let commit_messages: Vec<String> = commits.iter().map(|c| c.message.clone()).collect();

    let description = ai
        .generate_pr_description(&diff, &commit_messages)
        .await
        .map_err(|e| {
            tracing::error!("AI PR description failed: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("AI PR description failed: {}", e),
            )
        })?;

    Ok(Json(DescribePrResponse {
        title: description.title,
        body: description.body,
    }))
}

async fn ai_summarize_commit(
    State(state): State<AppState>,
    Path((owner, name, sha)): Path<(String, String, String)>,
    AuthUser(user): AuthUser,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let ai = require_ai(&state)?;
    let (_repo, _owner_user) = helpers::resolve_repo_authed(&state, &owner, &name, &user).await?;

    let repo_path = state
        .repo_host
        .repo_path(&owner, &name)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    let commit = delta_vcs::browse::show_commit(&repo_path, &sha)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;

    let summary = ai
        .generate_commit_summary(&commit.diff)
        .await
        .map_err(|e| {
            tracing::error!("AI commit summary failed: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("AI commit summary failed: {}", e),
            )
        })?;

    Ok(Json(serde_json::json!({
        "sha": sha,
        "summary": summary,
        "original_message": commit.message,
    })))
}

async fn ai_query_repo(
    State(state): State<AppState>,
    Path((owner, name)): Path<(String, String)>,
    AuthUser(user): AuthUser,
    Json(req): Json<QueryRequest>,
) -> Result<Json<QueryResponse>, (StatusCode, String)> {
    let ai = require_ai(&state)?;
    let (repo, _owner_user) = helpers::resolve_repo_authed(&state, &owner, &name, &user).await?;

    let repo_path = state
        .repo_host
        .repo_path(&owner, &name)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    // Gather context: top-level tree + recent commits + README if exists
    let tree = delta_vcs::browse::list_tree(&repo_path, "HEAD", "")
        .await
        .unwrap_or_default();

    let recent_commits = delta_vcs::browse::log(&repo_path, "HEAD", None, 10)
        .await
        .unwrap_or_default();

    // Try reading README
    let readme_content = delta_vcs::browse::read_blob_text(&repo_path, "HEAD", "README.md")
        .await
        .ok();

    // Search for relevant files if FTS5 is available
    let repo_id = repo.id.to_string();
    let search_results = db::search::search_repo(&state.db, &repo_id, &req.question, 5)
        .await
        .unwrap_or_default();

    let mut relevant_files: Vec<String> = search_results.iter().map(|r| r.path.clone()).collect();

    // Build context string
    let mut context = format!("Repository: {}/{}\n\n", owner, name);

    context.push_str("## File Tree\n");
    for entry in &tree {
        context.push_str(&format!("  {} {}\n", entry.kind, entry.path));
    }

    context.push_str("\n## Recent Commits\n");
    for commit in &recent_commits {
        context.push_str(&format!(
            "  {} - {} ({})\n",
            &commit.sha[..8.min(commit.sha.len())],
            commit.message,
            commit.author_name
        ));
    }

    if let Some(readme) = &readme_content {
        context.push_str("\n## README.md\n");
        // Truncate README if very long
        let readme_truncated = if readme.len() > 4000 {
            &readme[..4000]
        } else {
            readme
        };
        context.push_str(readme_truncated);
        context.push('\n');
    }

    if !search_results.is_empty() {
        context.push_str("\n## Relevant Code Snippets\n");
        for result in &search_results {
            context.push_str(&format!("### {}\n{}\n\n", result.path, result.snippet));
        }
    }

    // Try to read content of relevant files for more context
    for file_path in &relevant_files {
        if let Ok(content) = delta_vcs::browse::read_blob_text(&repo_path, "HEAD", file_path).await
        {
            let truncated = if content.len() > 2000 {
                &content[..2000]
            } else {
                &content
            };
            context.push_str(&format!(
                "\n## File: {}\n```\n{}\n```\n",
                file_path, truncated
            ));
        }
    }

    let answer = ai.query_repo(&req.question, &context).await.map_err(|e| {
        tracing::error!("AI query failed: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("AI query failed: {}", e),
        )
    })?;

    // Add file tree entries to relevant files if none found from search
    if relevant_files.is_empty() {
        relevant_files = tree.iter().take(10).map(|e| e.path.clone()).collect();
    }

    Ok(Json(QueryResponse {
        answer,
        relevant_files,
    }))
}

// ---------------------------------------------------------------------------
// Code search endpoints
// ---------------------------------------------------------------------------

async fn search_code(
    State(state): State<AppState>,
    Path((owner, name)): Path<(String, String)>,
    AuthUser(user): AuthUser,
    Query(query): Query<SearchQuery>,
) -> Result<Json<Vec<db::search::SearchResult>>, (StatusCode, String)> {
    let (repo, _owner_user) = helpers::resolve_repo_authed(&state, &owner, &name, &user).await?;
    let repo_id = repo.id.to_string();

    if query.q.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "search query is required".into()));
    }

    let limit = query.limit.min(100);

    let results = db::search::search_repo(&state.db, &repo_id, &query.q, limit)
        .await
        .map_err(|e| {
            tracing::error!("search failed: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("search failed: {}", e),
            )
        })?;

    Ok(Json(results))
}

async fn index_repo(
    State(state): State<AppState>,
    Path((owner, name)): Path<(String, String)>,
    AuthUser(user): AuthUser,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let (repo, owner_user) = helpers::resolve_repo_authed(&state, &owner, &name, &user).await?;

    // Only owner or admin can trigger indexing
    helpers::require_role(
        &state,
        &repo,
        &owner_user,
        &user,
        delta_core::models::collaborator::CollaboratorRole::Admin,
    )
    .await?;

    let repo_id = repo.id.to_string();
    let repo_path = state
        .repo_host
        .repo_path(&owner, &name)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    // Remove existing index for this repo
    db::search::remove_repo(&state.db, &repo_id)
        .await
        .map_err(|e| {
            tracing::error!("failed to clear search index: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "failed to clear search index".into(),
            )
        })?;

    // Walk the tree and index all text files
    let entries: Vec<String> = walk_tree_recursive(&repo_path, "HEAD", "").await;

    let mut indexed = 0u64;
    let mut skipped = 0u64;

    for entry_path in &entries {
        // Read file content
        match delta_vcs::browse::read_blob_text(&repo_path, "HEAD", entry_path).await {
            Ok(content) => {
                // Skip very large files (> 1MB) and likely binary files
                if content.len() > 1_048_576 || is_likely_binary(&content) {
                    skipped += 1;
                    continue;
                }
                if let Err(e) =
                    db::search::index_file(&state.db, &repo_id, entry_path, &content).await
                {
                    tracing::warn!("failed to index {}: {}", entry_path, e);
                    skipped += 1;
                } else {
                    indexed += 1;
                }
            }
            Err(_) => {
                skipped += 1;
            }
        }
    }

    Ok(Json(serde_json::json!({
        "indexed": indexed,
        "skipped": skipped,
        "total_files": entries.len(),
    })))
}

/// Recursively walk a git tree to get all blob paths.
fn walk_tree_recursive<'a>(
    repo_path: &'a std::path::Path,
    rev: &'a str,
    prefix: &'a str,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Vec<String>> + Send + 'a>> {
    Box::pin(async move {
        let entries = match delta_vcs::browse::list_tree(repo_path, rev, prefix).await {
            Ok(e) => e,
            Err(_) => return Vec::new(),
        };

        let mut paths = Vec::new();
        for entry in entries {
            if entry.kind == "blob" {
                paths.push(entry.path.clone());
            } else if entry.kind == "tree" {
                let sub = walk_tree_recursive(repo_path, rev, &entry.path).await;
                paths.extend(sub);
            }
        }
        paths
    })
}

/// Check if content is likely binary (contains null bytes).
fn is_likely_binary(content: &str) -> bool {
    content.as_bytes().iter().take(8192).any(|&b| b == 0)
}
