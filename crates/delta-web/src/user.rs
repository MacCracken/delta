//! User profile page template.

use askama::Template;

/// User profile data for display.
pub struct UserDisplay {
    pub username: String,
    pub display_name: Option<String>,
    pub is_agent: bool,
    pub created_at: String,
}

/// Repository entry for the profile page.
pub struct RepoEntry {
    pub name: String,
    pub description: Option<String>,
    pub visibility: String,
    pub updated_at: String,
}

/// User profile page.
#[derive(Template)]
#[template(path = "user/profile.html")]
pub struct ProfilePage {
    pub profile_user: UserDisplay,
    pub repos: Vec<RepoEntry>,
    pub repo_count: usize,
}
