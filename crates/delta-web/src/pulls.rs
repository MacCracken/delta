//! Pull request page templates.

use askama::Template;

// Import filters so Askama templates can reference custom filters
#[allow(unused_imports)]
use crate::filters;

/// PR list entry for display.
pub struct PullRequestEntry {
    pub number: i64,
    pub title: String,
    pub state: String,
    pub author_name: String,
    pub head_branch: String,
    pub base_branch: String,
    pub created_at: String,
}

/// Pull request list page.
#[derive(Template)]
#[template(path = "pulls/list.html")]
pub struct PullListPage {
    pub owner: String,
    pub repo: String,
    pub pulls: Vec<PullRequestEntry>,
    pub state_filter: String,
    pub open_count: usize,
    pub closed_count: usize,
}

/// PR detail display data.
pub struct PullRequestDisplay {
    pub number: i64,
    pub title: String,
    pub state: String,
    pub head_branch: String,
    pub base_branch: String,
    pub created_at: String,
}

/// Comment display data.
pub struct CommentDisplay {
    pub author_name: String,
    pub body: String,
    pub created_at: String,
    pub file_path: Option<String>,
    pub line: Option<i64>,
}

/// Review display data.
pub struct ReviewDisplay {
    pub reviewer_name: String,
    pub state: String,
    pub body: String,
    pub created_at: String,
}

/// Status check display data.
pub struct CheckDisplay {
    pub context: String,
    pub state: String,
    pub created_at: String,
}

/// Pull request detail page.
#[derive(Template)]
#[template(path = "pulls/detail.html")]
pub struct PullDetailPage {
    pub owner: String,
    pub repo: String,
    pub pr: PullRequestDisplay,
    pub author_name: String,
    pub comments: Vec<CommentDisplay>,
    pub reviews: Vec<ReviewDisplay>,
    pub diff: String,
    pub checks: Vec<CheckDisplay>,
    pub tab: String,
}
