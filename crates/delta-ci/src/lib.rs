//! CI/CD pipeline engine.
//!
//! Executes workflow definitions (`.delta/workflows/*.toml`) triggered by
//! repository events (push, PR, tag, schedule). Pipelines run in sandboxed
//! environments with configurable resource limits.

pub mod executor;
pub mod parser;
pub mod pipeline;
pub mod runner;
pub mod trigger;
pub mod workflow;

pub use pipeline::PipelineStatus;
pub use workflow::Workflow;
