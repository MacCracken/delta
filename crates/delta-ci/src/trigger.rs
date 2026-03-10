//! Trigger matching — determines which workflows to run for a given event.

use crate::workflow::{Trigger, Workflow};

/// Events that can trigger a workflow.
#[derive(Debug, Clone)]
pub enum Event {
    Push { branch: String },
    PullRequest { base_branch: String },
    Tag { tag_name: String },
    Manual,
}

/// Check if a workflow should be triggered by the given event.
pub fn should_trigger(workflow: &Workflow, event: &Event) -> bool {
    workflow.on.iter().any(|trigger| matches_trigger(trigger, event))
}

fn matches_trigger(trigger: &Trigger, event: &Event) -> bool {
    match (trigger, event) {
        (Trigger::Push { branches }, Event::Push { branch }) => {
            branches.iter().any(|b| pattern_matches(b, branch))
        }
        (Trigger::PullRequest { branches }, Event::PullRequest { base_branch }) => {
            branches.iter().any(|b| pattern_matches(b, base_branch))
        }
        (Trigger::Tag { pattern }, Event::Tag { tag_name }) => {
            pattern_matches(pattern, tag_name)
        }
        (Trigger::Schedule { .. }, _) => false, // Schedules are handled by cron, not events
        _ => false,
    }
}

/// Simple pattern matching: exact match or "*" wildcard suffix.
fn pattern_matches(pattern: &str, value: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if let Some(prefix) = pattern.strip_suffix('*') {
        return value.starts_with(prefix);
    }
    pattern == value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pattern_matching() {
        assert!(pattern_matches("main", "main"));
        assert!(!pattern_matches("main", "develop"));
        assert!(pattern_matches("*", "anything"));
        assert!(pattern_matches("release/*", "release/v1.0"));
        assert!(pattern_matches("release/", "release/"));
        assert!(!pattern_matches("release/*", "main"));
    }

    #[test]
    fn test_should_trigger_push() {
        let wf = Workflow {
            name: "CI".into(),
            on: vec![Trigger::Push {
                branches: vec!["main".into(), "develop".into()],
            }],
            jobs: Default::default(),
        };

        assert!(should_trigger(&wf, &Event::Push { branch: "main".into() }));
        assert!(should_trigger(&wf, &Event::Push { branch: "develop".into() }));
        assert!(!should_trigger(&wf, &Event::Push { branch: "feature/x".into() }));
    }

    #[test]
    fn test_should_trigger_tag() {
        let wf = Workflow {
            name: "Release".into(),
            on: vec![Trigger::Tag {
                pattern: "v*".into(),
            }],
            jobs: Default::default(),
        };

        assert!(should_trigger(&wf, &Event::Tag { tag_name: "v1.0".into() }));
        assert!(!should_trigger(&wf, &Event::Tag { tag_name: "release-1".into() }));
    }
}
