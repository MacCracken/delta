use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Branch protection rule for a repository.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchProtection {
    pub id: Uuid,
    pub repo_id: Uuid,
    /// Glob pattern matching branch names (e.g., "main", "release/*").
    pub pattern: String,
    /// Require pull request before merging.
    pub require_pr: bool,
    /// Minimum number of approving reviews required.
    pub required_approvals: u32,
    /// Require all status checks to pass before merging.
    pub require_status_checks: bool,
    /// Prevent force pushes.
    pub prevent_force_push: bool,
    /// Prevent branch deletion.
    pub prevent_deletion: bool,
}

impl BranchProtection {
    /// Check if a branch name matches this protection rule.
    pub fn matches(&self, branch: &str) -> bool {
        if self.pattern == branch {
            return true;
        }
        // Simple glob: "release/*" matches "release/2026.1.1"
        if let Some(prefix) = self.pattern.strip_suffix("/*") {
            return branch.starts_with(&format!("{}/", prefix));
        }
        false
    }

    /// Check if a push to this branch should be rejected.
    pub fn allows_direct_push(&self) -> bool {
        !self.require_pr
    }

    /// Check if force push is allowed.
    pub fn allows_force_push(&self) -> bool {
        !self.prevent_force_push
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_rule(pattern: &str, require_pr: bool, prevent_force: bool) -> BranchProtection {
        BranchProtection {
            id: Uuid::new_v4(),
            repo_id: Uuid::new_v4(),
            pattern: pattern.into(),
            require_pr,
            required_approvals: 1,
            require_status_checks: true,
            prevent_force_push: prevent_force,
            prevent_deletion: true,
        }
    }

    #[test]
    fn test_exact_match() {
        let rule = make_rule("main", false, false);
        assert!(rule.matches("main"));
        assert!(!rule.matches("develop"));
    }

    #[test]
    fn test_glob_match() {
        let rule = make_rule("release/*", false, false);
        assert!(rule.matches("release/2026.1.1"));
        assert!(rule.matches("release/beta"));
        assert!(!rule.matches("main"));
        assert!(!rule.matches("release"));
    }

    #[test]
    fn test_allows_direct_push() {
        assert!(make_rule("main", false, false).allows_direct_push());
        assert!(!make_rule("main", true, false).allows_direct_push());
    }

    #[test]
    fn test_allows_force_push() {
        assert!(make_rule("main", false, false).allows_force_push());
        assert!(!make_rule("main", false, true).allows_force_push());
    }
}
