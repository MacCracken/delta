//! Git-compatible version control backend.
//!
//! Handles bare repository management, git protocol (smart HTTP + SSH),
//! ref management, diff generation, and merge execution.

pub mod diff;
pub mod hosting;
pub mod merge;
pub mod protocol;
pub mod refs;

pub use hosting::RepoHost;
