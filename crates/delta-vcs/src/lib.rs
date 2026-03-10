//! Git-compatible version control backend.
//!
//! Handles bare repository management, git protocol (smart HTTP + SSH),
//! ref management, and object storage.

pub mod hosting;
pub mod protocol;
pub mod refs;

pub use hosting::RepoHost;
