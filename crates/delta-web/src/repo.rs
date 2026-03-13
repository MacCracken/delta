//! Repository page templates (file tree, blob viewer, blame, commits, commit detail).

use askama::Template;

use crate::filters;

/// Breadcrumb path component.
pub struct PathPart {
    pub name: String,
    pub url: String,
}

/// Display-ready tree entry.
pub struct TreeEntryDisplay {
    pub name: String,
    pub kind: String,
    pub path: String,
    pub last_commit_message: String,
    pub last_commit_date: String,
}

/// Repository file tree page.
#[derive(Template)]
#[template(path = "repo/tree.html")]
pub struct TreePage {
    pub owner: String,
    pub repo: String,
    pub rev: String,
    pub path: String,
    pub path_parts: Vec<PathPart>,
    pub entries: Vec<TreeEntryDisplay>,
    pub branches: Vec<String>,
    pub readme: Option<String>,
    pub is_empty: bool,
}

/// File blob viewer page.
#[derive(Template)]
#[template(path = "repo/blob.html")]
pub struct BlobPage {
    pub owner: String,
    pub repo: String,
    pub rev: String,
    pub path: String,
    pub path_parts: Vec<PathPart>,
    pub filename: String,
    pub content: String,
    pub line_count: usize,
    pub size_display: String,
    pub branches: Vec<String>,
}

/// Blame line for display.
pub struct BlameLineDisplay {
    pub sha: String,
    pub short_sha: String,
    pub author: String,
    pub date: String,
    pub line_number: usize,
    pub content: String,
    pub is_group_start: bool,
}

/// Blame viewer page.
#[derive(Template)]
#[template(path = "repo/blame.html")]
pub struct BlamePage {
    pub owner: String,
    pub repo: String,
    pub rev: String,
    pub path: String,
    pub path_parts: Vec<PathPart>,
    pub filename: String,
    pub lines: Vec<BlameLineDisplay>,
}

/// Commit log page.
#[derive(Template)]
#[template(path = "repo/commits.html")]
pub struct CommitsPage {
    pub owner: String,
    pub repo: String,
    pub rev: String,
    pub path: Option<String>,
    pub path_parts: Vec<PathPart>,
    pub commits: Vec<CommitEntry>,
}

/// Commit entry for the log list.
pub struct CommitEntry {
    pub sha: String,
    pub message: String,
    pub author_name: String,
    pub date: String,
}

/// File stat for commit detail.
pub struct CommitFileStatDisplay {
    pub path: String,
    pub additions: usize,
    pub deletions: usize,
}

/// Commit detail display data.
pub struct CommitDetailDisplay {
    pub sha: String,
    pub author_name: String,
    pub author_email: String,
    pub author_date: String,
    pub committer_name: String,
    pub committer_email: String,
    pub committer_date: String,
    pub parents: Vec<String>,
    pub message: String,
    pub body: String,
}

/// Commit detail page.
#[derive(Template)]
#[template(path = "repo/commit.html")]
pub struct CommitPage {
    pub owner: String,
    pub repo: String,
    pub commit: CommitDetailDisplay,
    pub diff_html: String,
    pub stats: Vec<CommitFileStatDisplay>,
    pub total_additions: usize,
    pub total_deletions: usize,
}
