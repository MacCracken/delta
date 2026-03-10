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
    pub steps: Vec<Step>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Step {
    pub name: Option<String>,
    pub run: Option<String>,
    pub uses: Option<String>,
    pub with: HashMap<String, String>,
}
