//! Artifact and package registry.
//!
//! Stores release artifacts, container images, and packages produced by
//! CI/CD pipelines or uploaded directly. Supports content-addressable
//! storage with integrity verification.

pub mod ark;
pub mod artifact;
pub mod lfs_store;
pub mod oci;
pub mod retention;
pub mod signing;
pub mod store;

pub use artifact::Artifact;
pub use lfs_store::LfsStore;
pub use store::BlobStore;
