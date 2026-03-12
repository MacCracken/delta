//! Workflow file parser.
//!
//! Reads `.delta/workflows/*.toml` files from a repository and parses
//! them into Workflow structs. Supports reusable job templates stored
//! in `.delta/templates/*.toml`.

use crate::workflow::{JobTemplate, Workflow};
use std::path::Path;

const MAX_FILE_SIZE: u64 = 1_048_576; // 1 MB

/// Parse a workflow from a TOML string.
pub fn parse_workflow(toml_str: &str) -> Result<Workflow, toml::de::Error> {
    toml::from_str(toml_str)
}

/// Parse a job template from a TOML string.
pub fn parse_template(toml_str: &str) -> Result<JobTemplate, toml::de::Error> {
    toml::from_str(toml_str)
}

/// Load a template by name from `.delta/templates/{name}.toml`.
fn load_template(repo_path: &Path, template_name: &str) -> Option<JobTemplate> {
    // Validate template name to prevent path traversal
    if template_name.contains('/')
        || template_name.contains('\\')
        || template_name.contains("..")
        || template_name.starts_with('.')
    {
        tracing::warn!(template_name, "rejected template name (path traversal)");
        return None;
    }

    let path = repo_path
        .join(".delta")
        .join("templates")
        .join(format!("{}.toml", template_name));

    if let Ok(meta) = path.metadata()
        && meta.len() > MAX_FILE_SIZE
    {
        tracing::warn!(path = %path.display(), "skipping oversized template file");
        return None;
    }

    let content = std::fs::read_to_string(&path).ok()?;
    match parse_template(&content) {
        Ok(tpl) => Some(tpl),
        Err(e) => {
            tracing::warn!(
                path = %path.display(),
                "failed to parse template: {}", e
            );
            None
        }
    }
}

/// Resolve template references in a workflow's jobs.
/// For each job with a `uses` field, load the template and merge its
/// steps/config into the job. The job's own fields take precedence.
pub fn resolve_templates(repo_path: &Path, workflow: &mut Workflow) {
    for (_job_key, job) in workflow.jobs.iter_mut() {
        let template_name = match &job.uses {
            Some(name) => name.clone(),
            None => continue,
        };

        let Some(template) = load_template(repo_path, &template_name) else {
            tracing::warn!(
                template = template_name,
                "template not found, skipping resolution"
            );
            continue;
        };

        // Merge: template provides defaults, job overrides
        if job.name.is_none() {
            job.name = template.name;
        }
        if job.runs_on.is_none() {
            job.runs_on = template.runs_on;
        }
        if job.steps.is_empty() {
            // Use template steps, injecting job-level `with` params into each step
            let mut steps = template.steps;
            if !job.with.is_empty() {
                for step in &mut steps {
                    for (k, v) in &job.with {
                        step.with.entry(k.clone()).or_insert_with(|| v.clone());
                    }
                }
            }
            job.steps = steps;
        }

        // Clear the uses field after resolution
        job.uses = None;
    }
}

/// Load all workflows from a repository's `.delta/workflows/` directory.
/// Automatically resolves template references.
pub fn load_workflows(repo_path: &Path) -> Vec<(String, Workflow)> {
    let workflows_dir = repo_path.join(".delta").join("workflows");
    if !workflows_dir.exists() {
        return vec![];
    }

    let mut workflows = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&workflows_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            // Skip files larger than 1 MB to prevent DoS
            if let Ok(meta) = path.metadata()
                && meta.len() > MAX_FILE_SIZE
            {
                tracing::warn!(path = %path.display(), "skipping oversized workflow file");
                continue;
            }
            if path.extension().is_some_and(|e| e == "toml")
                && let Ok(content) = std::fs::read_to_string(&path)
            {
                match parse_workflow(&content) {
                    Ok(mut wf) => {
                        resolve_templates(repo_path, &mut wf);
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
    use std::fs;
    use tempfile::TempDir;

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

    #[test]
    fn test_parse_template() {
        let toml = r#"
name = "Rust Test"
runs_on = "linux"

[[steps]]
name = "Checkout"
run = "git checkout $COMMIT_SHA"

[[steps]]
name = "Test"
run = "cargo test"
"#;
        let tpl = parse_template(toml).unwrap();
        assert_eq!(tpl.name.as_deref(), Some("Rust Test"));
        assert_eq!(tpl.steps.len(), 2);
    }

    #[test]
    fn test_resolve_templates() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();

        // Create template
        let templates_dir = root.join(".delta").join("templates");
        fs::create_dir_all(&templates_dir).unwrap();
        fs::write(
            templates_dir.join("rust-test.toml"),
            r#"
name = "Rust Test Suite"
runs_on = "linux"

[[steps]]
name = "Checkout"
run = "git checkout $COMMIT_SHA"

[[steps]]
name = "Test"
run = "cargo test"
"#,
        )
        .unwrap();

        // Create workflow referencing the template
        let workflows_dir = root.join(".delta").join("workflows");
        fs::create_dir_all(&workflows_dir).unwrap();
        fs::write(
            workflows_dir.join("ci.toml"),
            r#"
name = "CI"

[[on]]
push = { branches = ["main"] }

[jobs.test]
uses = "rust-test"
needs = []

[jobs.lint]
name = "Lint"
needs = ["test"]

[[jobs.lint.steps]]
name = "Clippy"
run = "cargo clippy"
"#,
        )
        .unwrap();

        let workflows = load_workflows(root);
        assert_eq!(workflows.len(), 1);

        let (_, wf) = &workflows[0];
        let test_job = &wf.jobs["test"];
        // Template name resolved
        assert_eq!(test_job.name.as_deref(), Some("Rust Test Suite"));
        assert_eq!(test_job.runs_on.as_deref(), Some("linux"));
        assert_eq!(test_job.steps.len(), 2);
        assert!(test_job.uses.is_none()); // cleared after resolution

        // Non-template job unchanged
        let lint_job = &wf.jobs["lint"];
        assert_eq!(lint_job.steps.len(), 1);
    }

    #[test]
    fn test_template_with_params() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();

        let templates_dir = root.join(".delta").join("templates");
        fs::create_dir_all(&templates_dir).unwrap();
        fs::write(
            templates_dir.join("build.toml"),
            r#"
name = "Build"

[[steps]]
name = "Build"
run = "cargo build --release"
"#,
        )
        .unwrap();

        let wf_toml = r#"
name = "Release"

[[on]]
tag = { pattern = "*" }

[jobs.build]
uses = "build"
needs = []

[jobs.build.with]
target = "x86_64-unknown-linux-gnu"
"#;

        let mut wf = parse_workflow(wf_toml).unwrap();
        resolve_templates(root, &mut wf);

        let build_job = &wf.jobs["build"];
        assert_eq!(build_job.steps.len(), 1);
        assert_eq!(
            build_job.steps[0].with.get("target").map(String::as_str),
            Some("x86_64-unknown-linux-gnu")
        );
    }

    #[test]
    fn test_template_path_traversal_rejected() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();

        let wf_toml = r#"
name = "Evil"

[[on]]
push = { branches = ["main"] }

[jobs.evil]
uses = "../../../etc/passwd"
needs = []
"#;

        let mut wf = parse_workflow(wf_toml).unwrap();
        resolve_templates(root, &mut wf);

        // Should not resolve — steps remain empty
        assert!(wf.jobs["evil"].steps.is_empty());
    }

    #[test]
    fn test_parse_matrix_workflow() {
        let toml = r#"
name = "Matrix CI"

[[on]]
push = { branches = ["main"] }

[jobs.test]
name = "Test"
needs = []

[jobs.test.strategy]
fail_fast = true

[jobs.test.strategy.matrix]
os = ["linux", "macos"]
rust = ["stable", "nightly"]

[[jobs.test.steps]]
name = "Run tests"
run = "cargo test"
"#;

        let wf = parse_workflow(toml).unwrap();
        let test_job = &wf.jobs["test"];
        let strategy = test_job.strategy.as_ref().unwrap();
        assert!(strategy.fail_fast);
        assert_eq!(strategy.matrix.len(), 2);
        assert_eq!(strategy.matrix["os"], vec!["linux", "macos"]);
        assert_eq!(strategy.matrix["rust"], vec!["stable", "nightly"]);
    }
}
