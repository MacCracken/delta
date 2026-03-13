use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeltaConfig {
    pub server: ServerConfig,
    pub storage: StorageConfig,
    pub auth: AuthConfig,
    #[serde(default)]
    pub registry: RegistryConfig,
    #[serde(default)]
    pub webhooks: WebhookConfig,
    #[serde(default)]
    pub ssh: SshConfig,
    #[serde(default)]
    pub ci: CiConfig,
    #[serde(default)]
    pub ai: AiConfig,
    #[serde(default)]
    pub agnos: AgnosConfig,
    #[serde(default)]
    pub federation: FederationConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub api_prefix: String,
    /// Allowed CORS origins. Empty list means allow any origin (dev only).
    #[serde(default)]
    pub cors_origins: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageConfig {
    pub repos_dir: PathBuf,
    pub artifacts_dir: PathBuf,
    pub db_url: String,
    /// Directory for LFS object storage. Defaults to `{artifacts_dir}/lfs`.
    #[serde(default)]
    pub lfs_dir: Option<PathBuf>,
}

impl StorageConfig {
    /// LFS storage directory, defaulting to `{artifacts_dir}/lfs`.
    pub fn lfs_dir(&self) -> PathBuf {
        self.lfs_dir
            .clone()
            .unwrap_or_else(|| self.artifacts_dir.join("lfs"))
    }
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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RegistryConfig {
    /// Maximum age in days before artifacts are eligible for cleanup.
    pub max_artifact_age_days: Option<u32>,
    /// Maximum number of artifacts per repository.
    pub max_artifacts_per_repo: Option<u32>,
    /// Maximum total artifact bytes per repository.
    pub max_total_bytes_per_repo: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WebhookConfig {
    /// When true, only HTTPS webhook URLs are allowed. HTTP URLs are rejected.
    #[serde(default)]
    pub https_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SshConfig {
    /// Enable the built-in SSH server.
    #[serde(default)]
    pub enabled: bool,
    /// SSH listen port (default: 2222).
    #[serde(default = "default_ssh_port")]
    pub port: u16,
    /// Path to the host ed25519 private key file.
    /// If not set, a key is generated at `{repos_dir}/../ssh_host_ed25519_key`.
    pub host_key_file: Option<String>,
}

impl Default for SshConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            port: 2222,
            host_key_file: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CiConfig {
    /// Enable sandboxed step execution (Landlock + seccomp on Linux).
    #[serde(default = "default_sandbox_enabled")]
    pub sandbox_enabled: bool,
    /// Container runtime to use when kernel sandboxing is unavailable.
    #[serde(default)]
    pub container_runtime: ContainerRuntime,
}

impl Default for CiConfig {
    fn default() -> Self {
        Self {
            sandbox_enabled: true,
            container_runtime: ContainerRuntime::Auto,
        }
    }
}

fn default_sandbox_enabled() -> bool {
    true
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ContainerRuntime {
    /// Automatically detect podman or docker.
    #[default]
    Auto,
    Podman,
    Docker,
    /// Disable container fallback.
    None,
}

fn default_ssh_port() -> u16 {
    2222
}

fn default_secrets_key() -> String {
    "delta-change-me-in-production".into()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgnosConfig {
    /// Enable AGNOS integration (daimon registration, etc.)
    #[serde(default)]
    pub enabled: bool,
    /// Daimon agent runtime URL for capability registration.
    #[serde(default = "default_daimon_url")]
    pub daimon_url: String,
}

fn default_daimon_url() -> String {
    "http://localhost:8090".into()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FederationConfig {
    /// Enable federation with other Delta instances.
    #[serde(default)]
    pub enabled: bool,
    /// This instance's public URL (used in federation handshakes).
    #[serde(default)]
    pub instance_url: Option<String>,
    /// Human-readable instance name.
    #[serde(default)]
    pub instance_name: Option<String>,
    /// Timeout for federation HTTP requests in seconds.
    #[serde(default = "default_federation_timeout")]
    pub timeout_secs: u64,
}

fn default_federation_timeout() -> u64 {
    30
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AiConfig {
    /// Enable AI-powered features (code review, PR summaries, etc.)
    #[serde(default)]
    pub enabled: bool,
    /// LLM provider configuration
    #[serde(default)]
    pub provider: AiProvider,
    /// API key for the LLM provider
    #[serde(default)]
    pub api_key: Option<String>,
    /// Model name to use (e.g. "claude-sonnet-4-20250514")
    #[serde(default = "default_model")]
    pub model: String,
    /// Maximum tokens in LLM response
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
    /// Custom endpoint URL (used by Hoosh provider, defaults to http://localhost:8088)
    #[serde(default)]
    pub endpoint: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AiProvider {
    #[default]
    Anthropic,
    OpenAI,
    Hoosh,
}

fn default_model() -> String {
    "claude-sonnet-4-20250514".into()
}

fn default_max_tokens() -> u32 {
    4096
}

impl Default for DeltaConfig {
    fn default() -> Self {
        Self {
            server: ServerConfig {
                host: "127.0.0.1".into(),
                port: 8070,
                api_prefix: "/api/v1".into(),
                cors_origins: vec![],
            },
            storage: StorageConfig {
                repos_dir: PathBuf::from("/var/lib/delta/repos"),
                artifacts_dir: PathBuf::from("/var/lib/delta/artifacts"),
                db_url: "sqlite:///var/lib/delta/delta.db?mode=rwc".into(),
                lfs_dir: None,
            },
            auth: AuthConfig {
                enabled: true,
                token_expiry_secs: 86400,
                secrets_key: default_secrets_key(),
            },
            registry: RegistryConfig::default(),
            webhooks: WebhookConfig::default(),
            ssh: SshConfig::default(),
            ci: CiConfig::default(),
            ai: AiConfig::default(),
            agnos: AgnosConfig::default(),
            federation: FederationConfig::default(),
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
        assert!(config.server.cors_origins.is_empty());
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
    fn test_config_cors_origins_from_toml() {
        let toml_str = r#"
[server]
host = "0.0.0.0"
port = 8070
api_prefix = "/api/v1"
cors_origins = ["https://delta.example.com", "https://admin.example.com"]

[storage]
repos_dir = "/tmp/repos"
artifacts_dir = "/tmp/artifacts"
db_url = "sqlite:///tmp/test.db"

[auth]
enabled = true
token_expiry_secs = 86400
"#;
        let config: DeltaConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.server.cors_origins.len(), 2);
        assert_eq!(config.server.cors_origins[0], "https://delta.example.com");
    }

    #[test]
    fn test_config_cors_origins_defaults_empty() {
        let toml_str = r#"
[server]
host = "0.0.0.0"
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
        assert!(config.server.cors_origins.is_empty());
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
