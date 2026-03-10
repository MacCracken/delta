use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A stored artifact (release binary, package, container image, etc.).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    pub id: Uuid,
    pub repo_owner: String,
    pub repo_name: String,
    pub name: String,
    pub version: String,
    pub content_hash: String,
    pub size_bytes: u64,
    pub artifact_type: ArtifactType,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactType {
    Binary,
    Archive,
    Container,
    ArkPackage,
    Generic,
}
