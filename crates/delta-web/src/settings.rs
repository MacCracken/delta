//! Repository settings page template.

use askama::Template;

/// Collaborator entry for settings display.
pub struct CollaboratorEntry {
    pub username: String,
    pub role: String,
}

/// Branch protection rule for settings display.
pub struct ProtectionEntry {
    pub branch_pattern: String,
    pub required_approvals: u32,
}

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
    pub collaborators: Vec<CollaboratorEntry>,
    pub protections: Vec<ProtectionEntry>,
}
