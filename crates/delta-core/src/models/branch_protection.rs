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
    ///
    /// Patterns are globs: `*` matches any run of characters (including `/`,
    /// so `release/*` also covers `release/1/hotfix`) and `?` matches one
    /// character. Everything else matches literally.
    pub fn matches(&self, branch: &str) -> bool {
        glob_match(self.pattern.as_bytes(), branch.as_bytes())
    }

    /// Combine every rule matching `branch` into one, taking the most
    /// restrictive value of each setting. Returns `None` if none match.
    pub fn effective<'a>(
        rules: impl IntoIterator<Item = &'a BranchProtection>,
        branch: &str,
    ) -> Option<BranchProtection> {
        rules
            .into_iter()
            .filter(|r| r.matches(branch))
            .cloned()
            .reduce(|mut acc, r| {
                acc.require_pr |= r.require_pr;
                acc.required_approvals = acc.required_approvals.max(r.required_approvals);
                acc.require_status_checks |= r.require_status_checks;
                acc.prevent_force_push |= r.prevent_force_push;
                acc.prevent_deletion |= r.prevent_deletion;
                acc
            })
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

/// Glob match supporting `*` (any run of bytes) and `?` (any single byte).
fn glob_match(pattern: &[u8], text: &[u8]) -> bool {
    let (mut p, mut t) = (0, 0);
    // Position of the last `*` in the pattern and the text index it matched up to.
    let mut star: Option<(usize, usize)> = None;
    while t < text.len() {
        match pattern.get(p) {
            Some(b'*') => {
                star = Some((p, t));
                p += 1;
            }
            Some(&c) if c == b'?' || c == text[t] => {
                p += 1;
                t += 1;
            }
            _ => match star {
                // Let the last `*` absorb one more byte and retry.
                Some((sp, st)) => {
                    p = sp + 1;
                    t = st + 1;
                    star = Some((sp, st + 1));
                }
                None => return false,
            },
        }
    }
    pattern[p..].iter().all(|&c| c == b'*')
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
    fn test_wildcard_patterns() {
        assert!(make_rule("*", false, false).matches("anything/at/all"));
        let feature = make_rule("feature-*", false, false);
        assert!(feature.matches("feature-login"));
        assert!(!feature.matches("bugfix-login"));
        assert!(make_rule("release/*", false, false).matches("release/1/hotfix"));
        assert!(make_rule("v?.x", false, false).matches("v2.x"));
        assert!(!make_rule("v?.x", false, false).matches("v10.x"));
        assert!(make_rule("*-stable", false, false).matches("2026-stable"));
    }

    #[test]
    fn test_effective_combines_most_restrictive() {
        let mut loose = make_rule("release/*", false, false);
        loose.required_approvals = 0;
        loose.prevent_deletion = false;
        let mut strict = make_rule("release/v1", true, true);
        strict.required_approvals = 2;

        // Rule order must not matter.
        for rules in [vec![&loose, &strict], vec![&strict, &loose]] {
            let rule = BranchProtection::effective(rules, "release/v1").unwrap();
            assert!(rule.require_pr);
            assert!(rule.prevent_force_push);
            assert!(rule.prevent_deletion);
            assert_eq!(rule.required_approvals, 2);
        }
        let other = BranchProtection::effective([&loose, &strict], "release/v2").unwrap();
        assert!(!other.require_pr);
        assert!(BranchProtection::effective([&loose, &strict], "main").is_none());
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
