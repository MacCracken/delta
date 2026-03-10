//! Artifact and package registry.
//!
//! Stores release artifacts, container images, and packages produced by
//! CI/CD pipelines or uploaded directly. Supports content-addressable
//! storage with integrity verification.

pub mod artifact;
pub mod store;

pub use artifact::Artifact;
pub use store::BlobStore;
