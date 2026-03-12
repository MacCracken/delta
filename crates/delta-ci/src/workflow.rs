use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A CI/CD workflow definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workflow {
    pub name: String,
    pub on: Vec<Trigger>,
    pub jobs: HashMap<String, Job>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Trigger {
    Push { branches: Vec<String> },
    PullRequest { branches: Vec<String> },
    Tag { pattern: String },
    Schedule { cron: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    pub name: Option<String>,
    pub runs_on: Option<String>,
    pub needs: Vec<String>,
    #[serde(default)]
    pub steps: Vec<Step>,
    /// Reference to a reusable template file, e.g. "rust-test" resolves to
    /// `.delta/templates/rust-test.toml`.
    pub uses: Option<String>,
    /// Parameters passed to the template, merged into step `with` maps.
    #[serde(default)]
    pub with: HashMap<String, String>,
    /// Matrix strategy — each key maps to a list of values. The job is
    /// expanded into one instance per combination of all dimensions.
    #[serde(default)]
    pub strategy: Option<MatrixStrategy>,
}

/// Matrix build strategy. Produces the Cartesian product of all dimension
/// values; each combination runs as an independent job instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatrixStrategy {
    /// Dimension name → list of values, e.g. `os = ["linux", "macos"]`.
    pub matrix: HashMap<String, Vec<String>>,
    /// If true, cancel remaining matrix jobs when one fails (default: false).
    #[serde(default)]
    pub fail_fast: bool,
}

/// A reusable workflow template (`.delta/templates/*.toml`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobTemplate {
    pub name: Option<String>,
    pub runs_on: Option<String>,
    #[serde(default)]
    pub steps: Vec<Step>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Step {
    pub name: Option<String>,
    pub run: Option<String>,
    pub uses: Option<String>,
    #[serde(default)]
    pub with: HashMap<String, String>,
}
