//! Repository settings page template.

use askama::Template;

/// Repository settings page.
#[derive(Template)]
#[template(path = "settings/repo.html")]
pub struct RepoSettingsPage {
    pub owner: String,
    pub repo: String,
    pub description: Option<String>,
    pub visibility: String,
    pub default_branch: String,
    pub branches: Vec<String>,
}
