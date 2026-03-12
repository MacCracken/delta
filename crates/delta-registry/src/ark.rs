//! Ark package types for AGNOS native packages.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArkPackageMeta {
    pub name: String,
    pub version: String,
    #[serde(default = "default_arch")]
    pub arch: String,
    pub description: Option<String>,
    #[serde(default)]
    pub dependencies: Vec<ArkDependency>,
    #[serde(default)]
    pub provides: Vec<String>,
}

fn default_arch() -> String {
    "any".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArkDependency {
    pub name: String,
    pub version_req: String,
}
