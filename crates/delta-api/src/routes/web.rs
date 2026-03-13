//! Server-rendered HTML pages for the web UI.

use askama::Template;
use axum::{
    Router,
    extract::{Path, Query, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use delta_core::db;
use serde::Deserialize;

use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        // Repository browsing
        .route("/{owner}/{repo}", get(repo_root))
        .route("/{owner}/{repo}/-/tree/{rev}", get(repo_tree_root))
        .route("/{owner}/{repo}/-/tree/{rev}/{*path}", get(repo_tree))
        .route("/{owner}/{repo}/-/blob/{rev}/{*path}", get(repo_blob))
        .route("/{owner}/{repo}/-/raw/{rev}/{*path}", get(repo_raw))
        .route("/{owner}/{repo}/-/blame/{rev}/{*path}", get(repo_blame))
        .route("/{owner}/{repo}/-/commits/{rev}", get(repo_commits))
        .route(
            "/{owner}/{repo}/-/commits/{rev}/{*path}",
            get(repo_commits_path),
        )
        .route("/{owner}/{repo}/-/commit/{sha}", get(repo_commit))
        // Pipelines (existing)
        .route("/{owner}/{repo}/-/pipelines", get(pipeline_list))
        .route(
            "/{owner}/{repo}/-/pipelines/{pipeline_id}",
            get(pipeline_detail),
        )
        // Pull requests
        .route("/{owner}/{repo}/-/pulls", get(pull_list))
        .route("/{owner}/{repo}/-/pulls/{number}", get(pull_detail))
        // Settings
        .route("/{owner}/{repo}/-/settings", get(repo_settings))
        // User profile
        .route("/{owner}", get(user_profile))
}

/// Render an Askama template into an HTML response.
fn render_template(tmpl: impl Template) -> Response {
    match tmpl.render() {
        Ok(html) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
            html,
        )
            .into_response(),
        Err(e) => {
            tracing::error!("template render error: {}", e);
            (StatusCode::INTERNAL_SERVER_ERROR, "internal server error").into_response()
        }
    }
}

type WebResult = Result<Response, (StatusCode, String)>;

fn internal_err(msg: &str) -> (StatusCode, String) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        msg.to_string(),
    )
}

fn not_found(msg: &str) -> (StatusCode, String) {
    (StatusCode::NOT_FOUND, msg.to_string())
}

/// Resolve repo and get the on-disk path.
async fn resolve_repo_path(
    state: &AppState,
    owner: &str,
    repo: &str,
) -> Result<(std::path::PathBuf, String), (StatusCode, String)> {
    let owner_user = db::user::get_by_username(&state.db, owner)
        .await
        .map_err(|_| not_found("user not found"))?;
    let owner_id = owner_user.id.to_string();
    let repo_record = db::repo::get_by_owner_and_name(&state.db, &owner_id, repo)
        .await
        .map_err(|_| not_found("repository not found"))?;
    let repo_path = state
        .repo_host
        .repo_path(owner, repo)
        .map_err(|e| internal_err(&format!("invalid repo path: {}", e)))?;
    Ok((repo_path, repo_record.id.to_string()))
}

/// Build path parts for breadcrumb navigation.
fn build_path_parts(owner: &str, repo: &str, rev: &str, path: &str) -> Vec<delta_web::repo::PathPart> {
    let mut parts = Vec::new();
    if path.is_empty() {
        return parts;
    }
    let components: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let mut accumulated = String::new();
    for (i, component) in components.iter().enumerate() {
        if !accumulated.is_empty() {
            accumulated.push('/');
        }
        accumulated.push_str(component);

        let url = if i == components.len() - 1 {
            // Last component — could be file or dir, link to tree for now
            format!("/{}/{}/-/tree/{}/{}", owner, repo, rev, accumulated)
        } else {
            format!("/{}/{}/-/tree/{}/{}", owner, repo, rev, accumulated)
        };
        parts.push(delta_web::repo::PathPart {
            name: component.to_string(),
            url,
        });
    }
    parts
}

fn format_size(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

// ---------------------------------------------------------------------------
// Repository browsing
// ---------------------------------------------------------------------------

async fn repo_root(
    State(state): State<AppState>,
    Path((owner, repo)): Path<(String, String)>,
) -> WebResult {
    // Redirect to tree view at default branch
    let owner_user = db::user::get_by_username(&state.db, &owner)
        .await
        .map_err(|_| not_found("user not found"))?;
    let owner_id = owner_user.id.to_string();
    let repo_record = db::repo::get_by_owner_and_name(&state.db, &owner_id, &repo)
        .await
        .map_err(|_| not_found("repository not found"))?;

    render_tree(&state, &owner, &repo, &repo_record.default_branch, "").await
}

async fn repo_tree_root(
    State(state): State<AppState>,
    Path((owner, repo, rev)): Path<(String, String, String)>,
) -> WebResult {
    render_tree(&state, &owner, &repo, &rev, "").await
}

async fn repo_tree(
    State(state): State<AppState>,
    Path((owner, repo, rev, path)): Path<(String, String, String, String)>,
) -> WebResult {
    render_tree(&state, &owner, &repo, &rev, &path).await
}

async fn render_tree(
    state: &AppState,
    owner: &str,
    repo: &str,
    rev: &str,
    path: &str,
) -> WebResult {
    let (repo_path, _repo_id) = resolve_repo_path(state, owner, repo).await?;

    // Check if repo is empty
    let head = delta_vcs::refs::head_commit(&repo_path);
    let is_empty = matches!(head, Ok(None) | Err(_));

    if is_empty {
        let page = delta_web::repo::TreePage {
            owner: owner.to_string(),
            repo: repo.to_string(),
            rev: rev.to_string(),
            path: String::new(),
            path_parts: Vec::new(),
            entries: Vec::new(),
            branches: Vec::new(),
            readme: None,
            is_empty: true,
        };
        return Ok(render_template(page));
    }

    let branches = delta_vcs::refs::list_branches(&repo_path)
        .unwrap_or_default()
        .into_iter()
        .map(|b| b.name)
        .collect::<Vec<_>>();

    let tree_entries = delta_vcs::browse::list_tree(&repo_path, rev, path)
        .await
        .map_err(|e| not_found(&format!("path not found: {}", e)))?;

    let entries: Vec<delta_web::repo::TreeEntryDisplay> = tree_entries
        .into_iter()
        .map(|e| delta_web::repo::TreeEntryDisplay {
            name: e.name,
            kind: e.kind,
            path: e.path,
            last_commit_message: String::new(),
            last_commit_date: String::new(),
        })
        .collect();

    // Check for README.md
    let readme = {
        let readme_path = if path.is_empty() {
            "README.md".to_string()
        } else {
            format!("{}/README.md", path.trim_end_matches('/'))
        };
        delta_vcs::browse::read_blob_text(&repo_path, rev, &readme_path)
            .await
            .ok()
    };

    let path_parts = build_path_parts(owner, repo, rev, path);

    let page = delta_web::repo::TreePage {
        owner: owner.to_string(),
        repo: repo.to_string(),
        rev: rev.to_string(),
        path: path.to_string(),
        path_parts,
        entries,
        branches,
        readme,
        is_empty: false,
    };

    Ok(render_template(page))
}

async fn repo_blob(
    State(state): State<AppState>,
    Path((owner, repo, rev, path)): Path<(String, String, String, String)>,
) -> WebResult {
    let (repo_path, _) = resolve_repo_path(&state, &owner, &repo).await?;

    let content = delta_vcs::browse::read_blob_text(&repo_path, &rev, &path)
        .await
        .map_err(|e| not_found(&format!("file not found: {}", e)))?;

    let line_count = content.lines().count();
    let size_display = format_size(content.len());
    let filename = path.rsplit('/').next().unwrap_or(&path).to_string();
    let path_parts = build_path_parts(&owner, &repo, &rev, &path);
    let branches = delta_vcs::refs::list_branches(&repo_path)
        .unwrap_or_default()
        .into_iter()
        .map(|b| b.name)
        .collect();

    let page = delta_web::repo::BlobPage {
        owner,
        repo,
        rev,
        path,
        path_parts,
        filename,
        content,
        line_count,
        size_display,
        branches,
    };

    Ok(render_template(page))
}

async fn repo_raw(
    State(state): State<AppState>,
    Path((owner, repo, rev, path)): Path<(String, String, String, String)>,
) -> WebResult {
    let (repo_path, _) = resolve_repo_path(&state, &owner, &repo).await?;

    let bytes = delta_vcs::browse::read_blob(&repo_path, &rev, &path)
        .await
        .map_err(|e| not_found(&format!("file not found: {}", e)))?;

    Ok((
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/octet-stream")],
        bytes,
    )
        .into_response())
}

async fn repo_blame(
    State(state): State<AppState>,
    Path((owner, repo, rev, path)): Path<(String, String, String, String)>,
) -> WebResult {
    let (repo_path, _) = resolve_repo_path(&state, &owner, &repo).await?;

    let blame_lines = delta_vcs::browse::blame(&repo_path, &rev, &path)
        .await
        .map_err(|e| not_found(&format!("blame failed: {}", e)))?;

    let filename = path.rsplit('/').next().unwrap_or(&path).to_string();
    let path_parts = build_path_parts(&owner, &repo, &rev, &path);

    // Build display lines with group_start detection
    let mut lines = Vec::with_capacity(blame_lines.len());
    let mut prev_sha = String::new();
    for bl in &blame_lines {
        let is_group_start = bl.sha != prev_sha;
        prev_sha.clone_from(&bl.sha);
        lines.push(delta_web::repo::BlameLineDisplay {
            short_sha: bl.sha.chars().take(8).collect(),
            sha: bl.sha.clone(),
            author: bl.author.clone(),
            date: bl.date.clone(),
            line_number: bl.line_number,
            content: bl.content.clone(),
            is_group_start,
        });
    }

    let page = delta_web::repo::BlamePage {
        owner,
        repo,
        rev,
        path,
        path_parts,
        filename,
        lines,
    };

    Ok(render_template(page))
}

async fn repo_commits(
    State(state): State<AppState>,
    Path((owner, repo, rev)): Path<(String, String, String)>,
) -> WebResult {
    render_commits(&state, &owner, &repo, &rev, None).await
}

async fn repo_commits_path(
    State(state): State<AppState>,
    Path((owner, repo, rev, path)): Path<(String, String, String, String)>,
) -> WebResult {
    render_commits(&state, &owner, &repo, &rev, Some(&path)).await
}

async fn render_commits(
    state: &AppState,
    owner: &str,
    repo: &str,
    rev: &str,
    path: Option<&str>,
) -> WebResult {
    let (repo_path, _) = resolve_repo_path(state, owner, repo).await?;

    let log_entries = delta_vcs::browse::log(&repo_path, rev, path, 100)
        .await
        .map_err(|e| not_found(&format!("commits not found: {}", e)))?;

    let commits: Vec<delta_web::repo::CommitEntry> = log_entries
        .into_iter()
        .map(|e| delta_web::repo::CommitEntry {
            sha: e.sha,
            message: e.message,
            author_name: e.author_name,
            date: e.date,
        })
        .collect();

    let path_parts = build_path_parts(owner, repo, rev, path.unwrap_or(""));

    let page = delta_web::repo::CommitsPage {
        owner: owner.to_string(),
        repo: repo.to_string(),
        rev: rev.to_string(),
        path: path.map(String::from),
        path_parts,
        commits,
    };

    Ok(render_template(page))
}

async fn repo_commit(
    State(state): State<AppState>,
    Path((owner, repo, sha)): Path<(String, String, String)>,
) -> WebResult {
    let (repo_path, _) = resolve_repo_path(&state, &owner, &repo).await?;

    let detail = delta_vcs::browse::show_commit(&repo_path, &sha)
        .await
        .map_err(|e| not_found(&format!("commit not found: {}", e)))?;

    let stats: Vec<delta_web::repo::CommitFileStatDisplay> = detail
        .stats
        .iter()
        .map(|s| delta_web::repo::CommitFileStatDisplay {
            path: s.path.clone(),
            additions: s.additions.unsigned_abs() as usize,
            deletions: s.deletions.unsigned_abs() as usize,
        })
        .collect();

    // Render diff HTML with syntax-highlighted lines
    let diff_html = render_diff_html(&detail.diff, &owner, &repo);

    let commit = delta_web::repo::CommitDetailDisplay {
        sha: detail.sha,
        author_name: detail.author_name,
        author_email: detail.author_email,
        author_date: detail.author_date,
        committer_name: detail.committer_name,
        committer_email: detail.committer_email,
        committer_date: detail.committer_date,
        parents: detail.parents,
        message: detail.message,
        body: detail.body,
    };

    let total_additions = stats.iter().map(|s| s.additions).sum();
    let total_deletions = stats.iter().map(|s| s.deletions).sum();

    let page = delta_web::repo::CommitPage {
        owner,
        repo,
        commit,
        diff_html,
        stats,
        total_additions,
        total_deletions,
    };

    Ok(render_template(page))
}

/// Render a unified diff into HTML with line numbers and coloring.
fn render_diff_html(diff: &str, _owner: &str, _repo: &str) -> String {
    use std::fmt::Write;
    let mut html = String::new();

    let mut current_file: Option<String> = None;
    let mut old_line: usize = 0;
    let mut new_line: usize = 0;
    let mut in_file = false;

    for line in diff.lines() {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            // Close previous file block
            if in_file {
                html.push_str("</table></div></div>");
            }

            // Extract file path from "a/path b/path"
            let file_path = rest
                .split(" b/")
                .nth(1)
                .unwrap_or(rest.trim_start_matches("a/"));

            current_file = Some(file_path.to_string());
            in_file = true;

            let _ = write!(
                html,
                "<div class=\"diff-file\"><div class=\"diff-file-header\">\
                 <span>{}</span></div>\
                 <div class=\"diff-viewer\"><table>",
                escape_html(file_path)
            );
            continue;
        }

        if line.starts_with("index ")
            || line.starts_with("--- ")
            || line.starts_with("+++ ")
            || line.starts_with("new file")
            || line.starts_with("deleted file")
            || line.starts_with("old mode")
            || line.starts_with("new mode")
            || line.starts_with("similarity index")
            || line.starts_with("rename from")
            || line.starts_with("rename to")
            || line.starts_with("Binary files")
        {
            continue;
        }

        if line.starts_with("@@") {
            // Parse hunk header: @@ -old,count +new,count @@
            if let Some((old, new)) = parse_hunk_header(line) {
                old_line = old;
                new_line = new;
            }
            let _ = write!(
                html,
                "<tr><td class=\"line-num-old\"></td><td class=\"line-num-new\"></td>\
                 <td class=\"diff-line diff-hunk\">{}</td></tr>",
                escape_html(line)
            );
            continue;
        }

        if current_file.is_none() {
            continue;
        }

        if let Some(rest) = line.strip_prefix('+') {
            let _ = write!(
                html,
                "<tr><td class=\"line-num-old\"></td><td class=\"line-num-new\">{}</td>\
                 <td class=\"diff-line diff-add\">+{}</td></tr>",
                new_line,
                escape_html(rest)
            );
            new_line += 1;
        } else if let Some(rest) = line.strip_prefix('-') {
            let _ = write!(
                html,
                "<tr><td class=\"line-num-old\">{}</td><td class=\"line-num-new\"></td>\
                 <td class=\"diff-line diff-del\">-{}</td></tr>",
                old_line,
                escape_html(rest)
            );
            old_line += 1;
        } else {
            // Context line (starts with space or is empty)
            let content = line.strip_prefix(' ').unwrap_or(line);
            let _ = write!(
                html,
                "<tr><td class=\"line-num-old\">{}</td><td class=\"line-num-new\">{}</td>\
                 <td class=\"diff-line diff-ctx\"> {}</td></tr>",
                old_line,
                new_line,
                escape_html(content)
            );
            old_line += 1;
            new_line += 1;
        }
    }

    if in_file {
        html.push_str("</table></div></div>");
    }

    html
}

fn parse_hunk_header(line: &str) -> Option<(usize, usize)> {
    // @@ -old_start[,count] +new_start[,count] @@
    let parts: Vec<&str> = line.split_whitespace().collect();
    if parts.len() < 3 {
        return None;
    }
    let old_start = parts[1]
        .trim_start_matches('-')
        .split(',')
        .next()?
        .parse::<usize>()
        .ok()?;
    let new_start = parts[2]
        .trim_start_matches('+')
        .split(',')
        .next()?
        .parse::<usize>()
        .ok()?;
    Some((old_start, new_start))
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

// ---------------------------------------------------------------------------
// Pipelines (existing)
// ---------------------------------------------------------------------------

async fn pipeline_list(
    State(state): State<AppState>,
    Path((owner, repo)): Path<(String, String)>,
) -> WebResult {
    let owner_user = db::user::get_by_username(&state.db, &owner)
        .await
        .map_err(|_| not_found("user not found"))?;
    let owner_id = owner_user.id.to_string();
    let repo_record = db::repo::get_by_owner_and_name(&state.db, &owner_id, &repo)
        .await
        .map_err(|_| not_found("repository not found"))?;

    let pipelines =
        db::pipeline::list_pipelines(&state.db, &repo_record.id.to_string(), None, 50)
            .await
            .map_err(|e| {
                tracing::error!("failed to list pipelines: {}", e);
                internal_err("internal server error")
            })?;

    let page = delta_web::pipelines::PipelineListPage {
        owner,
        repo,
        pipelines,
    };

    Ok(render_template(page))
}

async fn pipeline_detail(
    State(state): State<AppState>,
    Path((owner, repo, pipeline_id)): Path<(String, String, String)>,
) -> WebResult {
    let owner_user = db::user::get_by_username(&state.db, &owner)
        .await
        .map_err(|_| not_found("user not found"))?;
    let owner_id = owner_user.id.to_string();
    let repo_record = db::repo::get_by_owner_and_name(&state.db, &owner_id, &repo)
        .await
        .map_err(|_| not_found("repository not found"))?;

    let pipeline = db::pipeline::get_pipeline(&state.db, &pipeline_id)
        .await
        .map_err(|e| not_found(&e.to_string()))?;

    if pipeline.repo_id != repo_record.id.to_string() {
        return Err(not_found("pipeline not found"));
    }

    let job_runs = db::pipeline::list_jobs(&state.db, &pipeline_id)
        .await
        .unwrap_or_default();

    let mut jobs = Vec::new();
    for job in job_runs {
        let steps = db::pipeline::get_step_logs(&state.db, &job.id)
            .await
            .unwrap_or_default();
        jobs.push(delta_web::pipelines::JobWithSteps { job, steps });
    }

    let page = delta_web::pipelines::PipelineDetailPage {
        owner,
        repo,
        pipeline,
        jobs,
    };

    Ok(render_template(page))
}

// ---------------------------------------------------------------------------
// Pull Requests
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct PullListQuery {
    state: Option<String>,
}

async fn pull_list(
    State(state): State<AppState>,
    Path((owner, repo)): Path<(String, String)>,
    Query(query): Query<PullListQuery>,
) -> WebResult {
    let owner_user = db::user::get_by_username(&state.db, &owner)
        .await
        .map_err(|_| not_found("user not found"))?;
    let owner_id = owner_user.id.to_string();
    let repo_record = db::repo::get_by_owner_and_name(&state.db, &owner_id, &repo)
        .await
        .map_err(|_| not_found("repository not found"))?;

    let repo_id = repo_record.id.to_string();
    let state_filter = query.state.unwrap_or_else(|| "open".to_string());

    let filter = if state_filter == "all" {
        None
    } else {
        Some(state_filter.as_str())
    };

    let all_prs = db::pull_request::list_for_repo(&state.db, &repo_id, filter)
        .await
        .unwrap_or_default();

    // Get counts
    let open_count = if state_filter == "open" {
        all_prs.len()
    } else {
        db::pull_request::list_for_repo(&state.db, &repo_id, Some("open"))
            .await
            .map(|v| v.len())
            .unwrap_or(0)
    };
    let closed_count = if state_filter == "closed" {
        all_prs.len()
    } else {
        db::pull_request::list_for_repo(&state.db, &repo_id, Some("closed"))
            .await
            .map(|v| v.len())
            .unwrap_or(0)
    };

    // Resolve author names
    let mut pulls = Vec::new();
    for pr in &all_prs {
        let author_name = db::user::get_by_id(&state.db, &pr.author_id.to_string())
            .await
            .map(|u| u.username)
            .unwrap_or_else(|_| "unknown".into());

        pulls.push(delta_web::pulls::PullRequestEntry {
            number: pr.number,
            title: pr.title.clone(),
            state: pr.state.as_str().to_string(),
            author_name,
            head_branch: pr.head_branch.clone(),
            base_branch: pr.base_branch.clone(),
            created_at: pr.created_at.format("%Y-%m-%d %H:%M").to_string(),
        });
    }

    let page = delta_web::pulls::PullListPage {
        owner,
        repo,
        pulls,
        state_filter,
        open_count,
        closed_count,
    };

    Ok(render_template(page))
}

#[derive(Deserialize)]
struct PullDetailQuery {
    tab: Option<String>,
}

async fn pull_detail(
    State(state): State<AppState>,
    Path((owner, repo, number)): Path<(String, String, i64)>,
    Query(query): Query<PullDetailQuery>,
) -> WebResult {
    let owner_user = db::user::get_by_username(&state.db, &owner)
        .await
        .map_err(|_| not_found("user not found"))?;
    let owner_id = owner_user.id.to_string();
    let repo_record = db::repo::get_by_owner_and_name(&state.db, &owner_id, &repo)
        .await
        .map_err(|_| not_found("repository not found"))?;

    let repo_id = repo_record.id.to_string();
    let pr = db::pull_request::get_by_number(&state.db, &repo_id, number)
        .await
        .map_err(|_| not_found("pull request not found"))?;

    let author_name = db::user::get_by_id(&state.db, &pr.author_id.to_string())
        .await
        .map(|u| u.username)
        .unwrap_or_else(|_| "unknown".into());

    let tab = query.tab.unwrap_or_else(|| "conversation".to_string());

    // Load comments
    let raw_comments = db::pull_request::list_comments(&state.db, &pr.id.to_string())
        .await
        .unwrap_or_default();
    let mut comments = Vec::new();
    for c in &raw_comments {
        let name = db::user::get_by_id(&state.db, &c.author_id.to_string())
            .await
            .map(|u| u.username)
            .unwrap_or_else(|_| "unknown".into());
        comments.push(delta_web::pulls::CommentDisplay {
            author_name: name,
            body: c.body.clone(),
            created_at: c.created_at.format("%Y-%m-%d %H:%M").to_string(),
            file_path: c.file_path.clone(),
            line: c.line,
        });
    }

    // Load reviews
    let raw_reviews = db::pull_request::list_reviews(&state.db, &pr.id.to_string())
        .await
        .unwrap_or_default();
    let mut reviews = Vec::new();
    for r in &raw_reviews {
        let name = db::user::get_by_id(&state.db, &r.reviewer_id.to_string())
            .await
            .map(|u| u.username)
            .unwrap_or_else(|_| "unknown".into());
        reviews.push(delta_web::pulls::ReviewDisplay {
            reviewer_name: name,
            state: r.state.as_str().to_string(),
            body: r.body.clone().unwrap_or_default(),
            created_at: r.created_at.format("%Y-%m-%d %H:%M").to_string(),
        });
    }

    // Load diff
    let diff = if tab == "diff" {
        let repo_path = state
            .repo_host
            .repo_path(&owner, &repo)
            .map_err(|e| internal_err(&e.to_string()))?;
        delta_vcs::diff::diff_refs(&repo_path, &pr.base_branch, &pr.head_branch)
            .await
            .unwrap_or_default()
    } else {
        String::new()
    };

    // Load status checks
    let checks = if tab == "checks" {
        if let Some(sha) = &pr.head_sha {
            db::status_check::get_for_commit(&state.db, &repo_id, sha)
                .await
                .unwrap_or_default()
                .into_iter()
                .map(|c| delta_web::pulls::CheckDisplay {
                    context: c.context,
                    state: c.state.as_str().to_string(),
                    created_at: c.created_at.format("%Y-%m-%d %H:%M").to_string(),
                })
                .collect()
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };

    let pr_display = delta_web::pulls::PullRequestDisplay {
        number: pr.number,
        title: pr.title.clone(),
        state: pr.state.as_str().to_string(),
        head_branch: pr.head_branch.clone(),
        base_branch: pr.base_branch.clone(),
        created_at: pr.created_at.format("%Y-%m-%d %H:%M").to_string(),
    };

    let page = delta_web::pulls::PullDetailPage {
        owner,
        repo,
        pr: pr_display,
        author_name,
        comments,
        reviews,
        diff,
        checks,
        tab,
    };

    Ok(render_template(page))
}

// ---------------------------------------------------------------------------
// User Profile
// ---------------------------------------------------------------------------

async fn user_profile(
    State(state): State<AppState>,
    Path(username): Path<String>,
) -> WebResult {
    let user = db::user::get_by_username(&state.db, &username)
        .await
        .map_err(|_| not_found("user not found"))?;

    let repos = db::repo::list_by_owner(&state.db, &user.id.to_string())
        .await
        .unwrap_or_default();

    let repo_entries: Vec<delta_web::user::RepoEntry> = repos
        .iter()
        .map(|r| delta_web::user::RepoEntry {
            name: r.name.clone(),
            description: r.description.clone(),
            visibility: r.visibility.as_str().to_string(),
            updated_at: r.updated_at.format("%Y-%m-%d").to_string(),
        })
        .collect();

    let repo_count = repo_entries.len();

    let profile_user = delta_web::user::UserDisplay {
        username: user.username,
        display_name: user.display_name,
        is_agent: user.is_agent,
        created_at: user.created_at.format("%Y-%m-%d").to_string(),
    };

    let page = delta_web::user::ProfilePage {
        profile_user,
        repos: repo_entries,
        repo_count,
    };

    Ok(render_template(page))
}

// ---------------------------------------------------------------------------
// Repository Settings
// ---------------------------------------------------------------------------

async fn repo_settings(
    State(state): State<AppState>,
    Path((owner, repo)): Path<(String, String)>,
) -> WebResult {
    let owner_user = db::user::get_by_username(&state.db, &owner)
        .await
        .map_err(|_| not_found("user not found"))?;
    let owner_id = owner_user.id.to_string();
    let repo_record = db::repo::get_by_owner_and_name(&state.db, &owner_id, &repo)
        .await
        .map_err(|_| not_found("repository not found"))?;

    let repo_id = repo_record.id.to_string();
    let repo_path = state
        .repo_host
        .repo_path(&owner, &repo)
        .map_err(|e| internal_err(&e.to_string()))?;

    let branches: Vec<String> = delta_vcs::refs::list_branches(&repo_path)
        .unwrap_or_default()
        .into_iter()
        .map(|b| b.name)
        .collect();

    // Collaborators
    let raw_collabs = db::collaborator::list_for_repo(&state.db, &repo_id)
        .await
        .unwrap_or_default();
    let mut collaborators = Vec::new();
    for c in &raw_collabs {
        let username = db::user::get_by_id(&state.db, &c.user_id.to_string())
            .await
            .map(|u| u.username)
            .unwrap_or_else(|_| "unknown".into());
        collaborators.push(delta_web::settings::CollaboratorEntry {
            username,
            role: format!("{:?}", c.role).to_lowercase(),
        });
    }

    // Branch protections
    let raw_protections = db::branch_protection::list_for_repo(&state.db, &repo_id)
        .await
        .unwrap_or_default();
    let protections: Vec<delta_web::settings::ProtectionEntry> = raw_protections
        .into_iter()
        .map(|p| delta_web::settings::ProtectionEntry {
            branch_pattern: p.pattern,
            required_approvals: p.required_approvals,
        })
        .collect();

    let page = delta_web::settings::RepoSettingsPage {
        owner,
        repo,
        description: repo_record.description,
        visibility: repo_record.visibility.as_str().to_string(),
        default_branch: repo_record.default_branch,
        branches,
        collaborators,
        protections,
    };

    Ok(render_template(page))
}
