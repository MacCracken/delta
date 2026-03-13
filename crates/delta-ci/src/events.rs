//! Pipeline event types for real-time log streaming.
//!
//! Events are broadcast through `tokio::sync::broadcast` channels, one per
//! active pipeline. WebSocket clients subscribe to receive live updates.

use dashmap::DashMap;
use serde::Serialize;
use std::sync::Arc;
use tokio::sync::broadcast;

/// An event emitted during pipeline execution.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PipelineEvent {
    JobStarted {
        job_name: String,
        job_id: String,
    },
    StepStarted {
        job_id: String,
        step_name: String,
        step_index: usize,
    },
    StepOutput {
        job_id: String,
        step_index: usize,
        line: String,
    },
    StepCompleted {
        job_id: String,
        step_index: usize,
        exit_code: i32,
    },
    JobCompleted {
        job_id: String,
        success: bool,
        exit_code: Option<i32>,
    },
    PipelineCompleted {
        status: String,
    },
}

/// Registry of active pipeline broadcast channels.
/// Key: pipeline_id, Value: broadcast sender for pipeline events.
pub type PipelineStreams = Arc<DashMap<String, broadcast::Sender<PipelineEvent>>>;

/// Create a new empty pipeline streams registry.
pub fn new_pipeline_streams() -> PipelineStreams {
    Arc::new(DashMap::new())
}
