use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeltaConfig {
    pub server: ServerConfig,
    pub storage: StorageConfig,
    pub auth: AuthConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub api_prefix: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageConfig {
    pub repos_dir: PathBuf,
    pub artifacts_dir: PathBuf,
    pub db_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthConfig {
    pub enabled: bool,
    pub token_expiry_secs: u64,
}

impl Default for DeltaConfig {
    fn default() -> Self {
        Self {
            server: ServerConfig {
                host: "127.0.0.1".into(),
                port: 8070,
                api_prefix: "/api/v1".into(),
            },
            storage: StorageConfig {
                repos_dir: PathBuf::from("/var/lib/delta/repos"),
                artifacts_dir: PathBuf::from("/var/lib/delta/artifacts"),
                db_url: "sqlite:///var/lib/delta/delta.db".into(),
            },
            auth: AuthConfig {
                enabled: true,
                token_expiry_secs: 86400,
            },
        }
    }
}
