//! Workflow file parser.
//!
//! Reads `.delta/workflows/*.toml` files from a repository and parses
//! them into Workflow structs.

use crate::workflow::Workflow;
use std::path::Path;

/// Parse a workflow from a TOML string.
pub fn parse_workflow(toml_str: &str) -> Result<Workflow, toml::de::Error> {
    toml::from_str(toml_str)
}

/// Load all workflows from a repository's `.delta/workflows/` directory.
pub fn load_workflows(repo_path: &Path) -> Vec<(String, Workflow)> {
    let workflows_dir = repo_path.join(".delta").join("workflows");
    if !workflows_dir.exists() {
        return vec![];
    }

    let mut workflows = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&workflows_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "toml")
                && let Ok(content) = std::fs::read_to_string(&path)
            {
                match parse_workflow(&content) {
                    Ok(wf) => {
                        let filename = path
                            .file_stem()
                            .map(|s| s.to_string_lossy().to_string())
                            .unwrap_or_default();
                        workflows.push((filename, wf));
                    }
                    Err(e) => {
                        tracing::warn!(
                            path = %path.display(),
                            "failed to parse workflow: {}",
                            e
                        );
                    }
                }
            }
        }
    }

    workflows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_workflow() {
        let toml = r#"
name = "CI"

[[on]]
push = { branches = ["main", "develop"] }

[jobs.test]
name = "Run tests"
needs = []

[[jobs.test.steps]]
name = "Checkout"
run = "git checkout $COMMIT_SHA"

[[jobs.test.steps]]
name = "Test"
run = "cargo test"

[jobs.lint]
name = "Lint"
needs = ["test"]

[[jobs.lint.steps]]
name = "Clippy"
run = "cargo clippy"
"#;

        let wf = parse_workflow(toml).unwrap();
        assert_eq!(wf.name, "CI");
        assert_eq!(wf.jobs.len(), 2);
        assert!(wf.jobs.contains_key("test"));
        assert!(wf.jobs.contains_key("lint"));
        assert_eq!(wf.jobs["lint"].needs, vec!["test"]);
        assert_eq!(wf.jobs["test"].steps.len(), 2);
    }
}
