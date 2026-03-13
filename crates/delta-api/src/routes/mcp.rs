//! MCP server — exposes Delta API as MCP tools for agnoshi shell.

use axum::{extract::State, http::StatusCode, routing::{get, post}, Json, Router};
use serde::{Deserialize, Serialize};

use crate::state::AppState;
use delta_core::db;

#[derive(Debug, Serialize)]
struct McpToolDescription {
    name: String,
    description: String,
    input_schema: serde_json::Value,
}

#[derive(Debug, Serialize)]
struct McpToolManifest {
    tools: Vec<McpToolDescription>,
}

#[derive(Debug, Deserialize)]
struct McpToolCall {
    name: String,
    arguments: serde_json::Value,
}

#[derive(Debug, Serialize)]
struct McpContentBlock {
    content_type: String,
    text: String,
}

#[derive(Debug, Serialize)]
struct McpToolResult {
    content: Vec<McpContentBlock>,
    is_error: bool,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/tools", get(list_tools))
        .route("/tools/call", post(call_tool))
}

async fn list_tools() -> Json<McpToolManifest> {
    Json(McpToolManifest {
        tools: vec![
            McpToolDescription {
                name: "delta_list_repos".into(),
                description: "List repositories, optionally filtered by owner username".into(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "owner": { "type": "string", "description": "Owner username to filter by" }
                    },
                    "required": []
                }),
            },
            McpToolDescription {
                name: "delta_get_repo".into(),
                description: "Get repository details by owner and name".into(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "owner": { "type": "string", "description": "Repository owner username" },
                        "name": { "type": "string", "description": "Repository name" }
                    },
                    "required": ["owner", "name"]
                }),
            },
            McpToolDescription {
                name: "delta_list_branches".into(),
                description: "List branches in a repository".into(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "owner": { "type": "string", "description": "Repository owner username" },
                        "name": { "type": "string", "description": "Repository name" }
                    },
                    "required": ["owner", "name"]
                }),
            },
            McpToolDescription {
                name: "delta_list_pulls".into(),
                description: "List pull requests for a repository".into(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "owner": { "type": "string", "description": "Repository owner username" },
                        "name": { "type": "string", "description": "Repository name" },
                        "state": { "type": "string", "description": "Filter by state: open, closed, merged" }
                    },
                    "required": ["owner", "name"]
                }),
            },
            McpToolDescription {
                name: "delta_get_pull".into(),
                description: "Get pull request details by number".into(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "owner": { "type": "string", "description": "Repository owner username" },
                        "name": { "type": "string", "description": "Repository name" },
                        "number": { "type": "integer", "description": "Pull request number" }
                    },
                    "required": ["owner", "name", "number"]
                }),
            },
            McpToolDescription {
                name: "delta_list_pipelines".into(),
                description: "List CI pipelines for a repository".into(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "owner": { "type": "string", "description": "Repository owner username" },
                        "name": { "type": "string", "description": "Repository name" }
                    },
                    "required": ["owner", "name"]
                }),
            },
            McpToolDescription {
                name: "delta_search_code".into(),
                description: "Search code across repositories".into(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "Search query string" },
                        "owner": { "type": "string", "description": "Repository owner username (scopes search to this owner's repo)" },
                        "name": { "type": "string", "description": "Repository name (requires owner)" }
                    },
                    "required": ["query"]
                }),
            },
            McpToolDescription {
                name: "delta_read_file".into(),
                description: "Read a file from a repository at a given ref".into(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "owner": { "type": "string", "description": "Repository owner username" },
                        "name": { "type": "string", "description": "Repository name" },
                        "path": { "type": "string", "description": "File path within the repository" },
                        "ref": { "type": "string", "description": "Git ref (branch, tag, or commit). Defaults to HEAD" }
                    },
                    "required": ["owner", "name", "path"]
                }),
            },
            McpToolDescription {
                name: "delta_list_tree".into(),
                description: "List directory contents in a repository".into(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "owner": { "type": "string", "description": "Repository owner username" },
                        "name": { "type": "string", "description": "Repository name" },
                        "path": { "type": "string", "description": "Directory path within the repository. Defaults to root" },
                        "ref": { "type": "string", "description": "Git ref (branch, tag, or commit). Defaults to HEAD" }
                    },
                    "required": ["owner", "name"]
                }),
            },
        ],
    })
}

async fn call_tool(
    State(state): State<AppState>,
    Json(call): Json<McpToolCall>,
) -> Result<Json<McpToolResult>, (StatusCode, Json<McpToolResult>)> {
    match call.name.as_str() {
        "delta_list_repos" => handle_list_repos(&state, &call.arguments).await,
        "delta_get_repo" => handle_get_repo(&state, &call.arguments).await,
        "delta_list_branches" => handle_list_branches(&state, &call.arguments).await,
        "delta_list_pulls" => handle_list_pulls(&state, &call.arguments).await,
        "delta_get_pull" => handle_get_pull(&state, &call.arguments).await,
        "delta_list_pipelines" => handle_list_pipelines(&state, &call.arguments).await,
        "delta_search_code" => handle_search_code(&state, &call.arguments).await,
        "delta_read_file" => handle_read_file(&state, &call.arguments).await,
        "delta_list_tree" => handle_list_tree(&state, &call.arguments).await,
        _ => Err(error_result(
            StatusCode::BAD_REQUEST,
            &format!("unknown tool: {}", call.name),
        )),
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

type ToolResult = Result<Json<McpToolResult>, (StatusCode, Json<McpToolResult>)>;

fn ok_json(value: &impl Serialize) -> ToolResult {
    let text = serde_json::to_string_pretty(value).unwrap_or_default();
    Ok(Json(McpToolResult {
        content: vec![McpContentBlock {
            content_type: "text".into(),
            text,
        }],
        is_error: false,
    }))
}

fn ok_text(text: String) -> ToolResult {
    Ok(Json(McpToolResult {
        content: vec![McpContentBlock {
            content_type: "text".into(),
            text,
        }],
        is_error: false,
    }))
}

fn error_result(status: StatusCode, msg: &str) -> (StatusCode, Json<McpToolResult>) {
    (
        status,
        Json(McpToolResult {
            content: vec![McpContentBlock {
                content_type: "text".into(),
                text: msg.to_string(),
            }],
            is_error: true,
        }),
    )
}

fn arg_str<'a>(args: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(|v| v.as_str())
}

fn require_str<'a>(args: &'a serde_json::Value, key: &str) -> Result<&'a str, (StatusCode, Json<McpToolResult>)> {
    arg_str(args, key)
        .ok_or_else(|| error_result(StatusCode::BAD_REQUEST, &format!("missing required argument: {}", key)))
}

/// Resolve an owner username to a user ID string.
async fn resolve_owner(state: &AppState, owner: &str) -> Result<String, (StatusCode, Json<McpToolResult>)> {
    let user = db::user::get_by_username(&state.db, owner)
        .await
        .map_err(|_| error_result(StatusCode::NOT_FOUND, &format!("user '{}' not found", owner)))?;
    Ok(user.id.to_string())
}

/// Resolve owner + repo name to a repo, returning the repo.
async fn resolve_repo(
    state: &AppState,
    owner: &str,
    name: &str,
) -> Result<delta_core::models::repo::Repository, (StatusCode, Json<McpToolResult>)> {
    let owner_id = resolve_owner(state, owner).await?;
    db::repo::get_by_owner_and_name(&state.db, &owner_id, name)
        .await
        .map_err(|_| error_result(StatusCode::NOT_FOUND, &format!("repository '{}/{}' not found", owner, name)))
}

// ---------------------------------------------------------------------------
// Tool handlers
// ---------------------------------------------------------------------------

async fn handle_list_repos(state: &AppState, args: &serde_json::Value) -> ToolResult {
    if let Some(owner) = arg_str(args, "owner") {
        let owner_id = resolve_owner(state, owner).await?;
        let repos = db::repo::list_by_owner(&state.db, &owner_id)
            .await
            .map_err(|e| error_result(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
        ok_json(&repos)
    } else {
        let repos = db::repo::list_visible(&state.db, None)
            .await
            .map_err(|e| error_result(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
        ok_json(&repos)
    }
}

async fn handle_get_repo(state: &AppState, args: &serde_json::Value) -> ToolResult {
    let owner = require_str(args, "owner")?;
    let name = require_str(args, "name")?;
    let repo = resolve_repo(state, owner, name).await?;
    ok_json(&repo)
}

async fn handle_list_branches(state: &AppState, args: &serde_json::Value) -> ToolResult {
    let owner = require_str(args, "owner")?;
    let name = require_str(args, "name")?;
    // Verify the repo exists in DB
    let _repo = resolve_repo(state, owner, name).await?;
    let repo_path = state.repo_host.repo_path(owner, name)
        .map_err(|e| error_result(StatusCode::BAD_REQUEST, &e.to_string()))?;
    let branches = delta_vcs::refs::list_branches(&repo_path)
        .map_err(|e| error_result(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    ok_json(&branches)
}

async fn handle_list_pulls(state: &AppState, args: &serde_json::Value) -> ToolResult {
    let owner = require_str(args, "owner")?;
    let name = require_str(args, "name")?;
    let state_filter = arg_str(args, "state");
    let repo = resolve_repo(state, owner, name).await?;
    let repo_id = repo.id.to_string();
    let pulls = db::pull_request::list_for_repo(&state.db, &repo_id, state_filter)
        .await
        .map_err(|e| error_result(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    ok_json(&pulls)
}

async fn handle_get_pull(state: &AppState, args: &serde_json::Value) -> ToolResult {
    let owner = require_str(args, "owner")?;
    let name = require_str(args, "name")?;
    let number = args.get("number")
        .and_then(|v| v.as_i64())
        .ok_or_else(|| error_result(StatusCode::BAD_REQUEST, "missing required argument: number"))?;
    let repo = resolve_repo(state, owner, name).await?;
    let repo_id = repo.id.to_string();
    let pr = db::pull_request::get_by_number(&state.db, &repo_id, number)
        .await
        .map_err(|e| error_result(StatusCode::NOT_FOUND, &e.to_string()))?;
    ok_json(&pr)
}

async fn handle_list_pipelines(state: &AppState, args: &serde_json::Value) -> ToolResult {
    let owner = require_str(args, "owner")?;
    let name = require_str(args, "name")?;
    let repo = resolve_repo(state, owner, name).await?;
    let repo_id = repo.id.to_string();
    let pipelines = db::pipeline::list_pipelines(&state.db, &repo_id, None, 50)
        .await
        .map_err(|e| error_result(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    ok_json(&pipelines)
}

async fn handle_search_code(state: &AppState, args: &serde_json::Value) -> ToolResult {
    let query = require_str(args, "query")?;
    let owner = arg_str(args, "owner");
    let name = arg_str(args, "name");

    if let (Some(owner), Some(name)) = (owner, name) {
        // Scoped search within a specific repo
        let repo = resolve_repo(state, owner, name).await?;
        let repo_id = repo.id.to_string();
        let results = db::search::search_repo(&state.db, &repo_id, query, 50)
            .await
            .map_err(|e| error_result(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
        ok_json(&results)
    } else {
        // Global search across all public repos
        let repos = db::repo::list_visible(&state.db, None)
            .await
            .map_err(|e| error_result(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
        let repo_ids: Vec<String> = repos.iter().map(|r| r.id.to_string()).collect();
        let results = db::search::search_global(&state.db, query, &repo_ids, 50)
            .await
            .map_err(|e| error_result(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
        ok_json(&results)
    }
}

async fn handle_read_file(state: &AppState, args: &serde_json::Value) -> ToolResult {
    let owner = require_str(args, "owner")?;
    let name = require_str(args, "name")?;
    let path = require_str(args, "path")?;
    let rev = arg_str(args, "ref").unwrap_or("HEAD");
    // Verify the repo exists
    let _repo = resolve_repo(state, owner, name).await?;
    let repo_path = state.repo_host.repo_path(owner, name)
        .map_err(|e| error_result(StatusCode::BAD_REQUEST, &e.to_string()))?;
    let content = delta_vcs::browse::read_blob_text(&repo_path, rev, path)
        .await
        .map_err(|e| error_result(StatusCode::NOT_FOUND, &e.to_string()))?;
    ok_text(content)
}

async fn handle_list_tree(state: &AppState, args: &serde_json::Value) -> ToolResult {
    let owner = require_str(args, "owner")?;
    let name = require_str(args, "name")?;
    let path = arg_str(args, "path").unwrap_or("");
    let rev = arg_str(args, "ref").unwrap_or("HEAD");
    // Verify the repo exists
    let _repo = resolve_repo(state, owner, name).await?;
    let repo_path = state.repo_host.repo_path(owner, name)
        .map_err(|e| error_result(StatusCode::BAD_REQUEST, &e.to_string()))?;
    let entries = delta_vcs::browse::list_tree(&repo_path, rev, path)
        .await
        .map_err(|e| error_result(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    ok_json(&entries)
}
