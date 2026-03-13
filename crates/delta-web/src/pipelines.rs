//! Pipeline page templates.

use askama::Template;
use delta_core::db::pipeline::{JobRun, PipelineRun, StepLog};

// Re-export filters module for Askama's custom filter resolution
use crate::filters;

/// Pipeline list page.
#[derive(Template)]
#[template(path = "pipelines/list.html")]
pub struct PipelineListPage {
    pub owner: String,
    pub repo: String,
    pub pipelines: Vec<PipelineRun>,
}

/// A job with its step logs (for the detail page).
pub struct JobWithSteps {
    pub job: JobRun,
    pub steps: Vec<StepLog>,
}

/// Pipeline detail page.
#[derive(Template)]
#[template(path = "pipelines/detail.html")]
pub struct PipelineDetailPage {
    pub owner: String,
    pub repo: String,
    pub pipeline: PipelineRun,
    pub jobs: Vec<JobWithSteps>,
}
