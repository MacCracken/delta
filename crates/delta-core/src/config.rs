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
    /// Passphrase used to derive the encryption key for pipeline secrets.
    /// If not set, a default key is derived (not secure for production).
    #[serde(default = "default_secrets_key")]
    pub secrets_key: String,
}

fn default_secrets_key() -> String {
    "delta-change-me-in-production".into()
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
                secrets_key: default_secrets_key(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = DeltaConfig::default();
        assert_eq!(config.server.host, "127.0.0.1");
        assert_eq!(config.server.port, 8070);
        assert!(config.auth.enabled);
        assert_eq!(config.auth.token_expiry_secs, 86400);
        assert!(!config.auth.secrets_key.is_empty());
    }

    #[test]
    fn test_config_from_toml() {
        let toml_str = r#"
[server]
host = "0.0.0.0"
port = 9090
api_prefix = "/api/v1"

[storage]
repos_dir = "/tmp/repos"
artifacts_dir = "/tmp/artifacts"
db_url = "sqlite:///tmp/test.db"

[auth]
enabled = false
token_expiry_secs = 3600
secrets_key = "test-key"
"#;
        let config: DeltaConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.server.host, "0.0.0.0");
        assert_eq!(config.server.port, 9090);
        assert!(!config.auth.enabled);
        assert_eq!(config.auth.token_expiry_secs, 3600);
        assert_eq!(config.auth.secrets_key, "test-key");
    }

    #[test]
    fn test_config_secrets_key_defaults() {
        let toml_str = r#"
[server]
host = "127.0.0.1"
port = 8070
api_prefix = "/api/v1"

[storage]
repos_dir = "/tmp/repos"
artifacts_dir = "/tmp/artifacts"
db_url = "sqlite:///tmp/test.db"

[auth]
enabled = true
token_expiry_secs = 86400
"#;
        let config: DeltaConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.auth.secrets_key, "delta-change-me-in-production");
    }
}
