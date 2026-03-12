//! Built-in SSH server for git transport.
//!
//! Accepts SSH connections, authenticates users via public key,
//! and delegates to the same git backend as HTTP transport.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use delta_core::db;
use delta_core::models::collaborator::CollaboratorRole;
use delta_core::models::repo::Visibility;
use russh::keys::{self as russh_keys, HashAlg, PrivateKey, PublicKey};
use russh::server::{Auth, Handler, Msg, Server, Session};
use russh::{Channel, ChannelId, CryptoVec};
use sqlx::SqlitePool;
use tokio::sync::Mutex;

/// Shared state for the SSH server.
#[derive(Clone)]
pub struct SshServerState {
    pub pool: SqlitePool,
    pub repos_dir: PathBuf,
}

/// The SSH server — creates a new handler per connection.
pub struct DeltaSshServer {
    pub state: SshServerState,
}

impl Server for DeltaSshServer {
    type Handler = SshSession;

    fn new_client(&mut self, _peer_addr: Option<std::net::SocketAddr>) -> Self::Handler {
        SshSession {
            state: self.state.clone(),
            user: None,
            channels: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

/// Per-connection session state.
pub struct SshSession {
    state: SshServerState,
    /// Authenticated user info: (user_id, username)
    user: Option<(String, String)>,
    channels: Arc<Mutex<HashMap<ChannelId, ChannelState>>>,
}

struct ChannelState {
    data: Vec<u8>,
    command: Option<String>,
}

impl Handler for SshSession {
    type Error = anyhow::Error;

    async fn auth_publickey(
        &mut self,
        _user: &str,
        public_key: &PublicKey,
    ) -> Result<Auth, Self::Error> {
        // Compute fingerprint using ssh-key's built-in method
        let fingerprint = public_key.fingerprint(HashAlg::Sha256).to_string();

        let result = db::ssh_key::get_user_by_fingerprint(&self.state.pool, &fingerprint).await;

        match result {
            Ok(Some((user_id, username))) => {
                tracing::info!(username = %username, "SSH auth success");
                self.user = Some((user_id, username));
                Ok(Auth::Accept)
            }
            _ => {
                tracing::debug!(fingerprint = %fingerprint, "SSH auth failed: key not found");
                Ok(Auth::reject())
            }
        }
    }

    async fn auth_password(
        &mut self,
        _user: &str,
        _password: &str,
    ) -> Result<Auth, Self::Error> {
        Ok(Auth::reject())
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<Msg>,
        _session: &mut Session,
    ) -> Result<bool, Self::Error> {
        if self.user.is_none() {
            return Ok(false);
        }
        self.channels.lock().await.insert(
            channel.id(),
            ChannelState {
                data: Vec::new(),
                command: None,
            },
        );
        Ok(true)
    }

    async fn exec_request(
        &mut self,
        channel_id: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let command = String::from_utf8_lossy(data).to_string();
        tracing::debug!(command = %command, "SSH exec request");

        // Parse git command: "git-upload-pack '/owner/repo.git'"
        let (service, repo_path) = match parse_git_command(&command) {
            Some(parsed) => parsed,
            None => {
                session.channel_failure(channel_id)?;
                return Ok(());
            }
        };

        let (owner, repo_name) = match parse_repo_path(&repo_path) {
            Some(parsed) => parsed,
            None => {
                let msg = format!("invalid repository path: {}\n", repo_path);
                session.data(channel_id, CryptoVec::from(msg.as_bytes()))?;
                session.close(channel_id)?;
                return Ok(());
            }
        };

        // Authorize
        let (user_id, _username) = match &self.user {
            Some(u) => u.clone(),
            None => {
                session.channel_failure(channel_id)?;
                return Ok(());
            }
        };

        if let Err(msg) = self
            .authorize(&service, &owner, &repo_name, &user_id)
            .await
        {
            let err = format!("ERROR: {}\n", msg);
            session.data(channel_id, CryptoVec::from(err.as_bytes()))?;
            session.close(channel_id)?;
            return Ok(());
        }

        // Store command for this channel
        if let Some(ch) = self.channels.lock().await.get_mut(&channel_id) {
            ch.command = Some(command.clone());
        }

        let disk_path = self
            .state
            .repos_dir
            .join(&owner)
            .join(format!("{}.git", repo_name));

        if !disk_path.exists() {
            let msg = format!("repository not found: {}/{}\n", owner, repo_name);
            session.data(channel_id, CryptoVec::from(msg.as_bytes()))?;
            session.close(channel_id)?;
            return Ok(());
        }

        // For upload-pack (clone/fetch), run immediately with empty stdin
        if service == "upload-pack" {
            let output = delta_vcs::protocol::upload_pack(&disk_path, &[]).await;
            match output {
                Ok(data) => {
                    session.data(channel_id, CryptoVec::from_slice(&data))?;
                }
                Err(e) => {
                    let msg = format!("git error: {}\n", e);
                    session.data(channel_id, CryptoVec::from(msg.as_bytes()))?;
                }
            }
            session.exit_status_request(channel_id, 0)?;
            session.eof(channel_id)?;
            session.close(channel_id)?;
        }
        // For receive-pack, data arrives via data() and completes on channel_eof()

        Ok(())
    }

    async fn data(
        &mut self,
        channel_id: ChannelId,
        data: &[u8],
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        let mut channels = self.channels.lock().await;
        if let Some(ch) = channels.get_mut(&channel_id) {
            ch.data.extend_from_slice(data);
        }
        Ok(())
    }

    async fn channel_eof(
        &mut self,
        channel_id: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let channels = self.channels.lock().await;
        let Some(ch) = channels.get(&channel_id) else {
            return Ok(());
        };

        let Some(command) = &ch.command else {
            return Ok(());
        };

        // Only handle receive-pack EOF (upload-pack already completed)
        if !command.contains("receive-pack") {
            return Ok(());
        }

        let (_, repo_path) = match parse_git_command(command) {
            Some(p) => p,
            None => return Ok(()),
        };
        let (owner, repo_name) = match parse_repo_path(&repo_path) {
            Some(p) => p,
            None => return Ok(()),
        };

        let disk_path = self
            .state
            .repos_dir
            .join(&owner)
            .join(format!("{}.git", repo_name));

        let input = ch.data.clone();
        drop(channels);

        let output = delta_vcs::protocol::receive_pack(&disk_path, &input).await;
        match output {
            Ok(data) => {
                session.data(channel_id, CryptoVec::from_slice(&data))?;
                session.exit_status_request(channel_id, 0)?;
            }
            Err(e) => {
                let msg = format!("git error: {}\n", e);
                session.data(channel_id, CryptoVec::from(msg.as_bytes()))?;
                session.exit_status_request(channel_id, 1)?;
            }
        }
        session.eof(channel_id)?;
        session.close(channel_id)?;

        Ok(())
    }
}

impl SshSession {
    async fn authorize(
        &self,
        service: &str,
        owner: &str,
        repo_name: &str,
        user_id: &str,
    ) -> Result<(), String> {
        let owner_user = db::user::get_by_username(&self.state.pool, owner)
            .await
            .map_err(|_| format!("user '{}' not found", owner))?;

        let owner_id = owner_user.id.to_string();
        let repo = db::repo::get_by_owner_and_name(&self.state.pool, &owner_id, repo_name)
            .await
            .map_err(|_| format!("repository '{}/{}' not found", owner, repo_name))?;

        let is_owner = user_id == owner_id;

        match service {
            "upload-pack" => {
                if repo.visibility == Visibility::Public || is_owner {
                    Ok(())
                } else {
                    let role = db::collaborator::get_role(
                        &self.state.pool,
                        &repo.id.to_string(),
                        user_id,
                    )
                    .await
                    .unwrap_or(None);
                    if role.is_some() {
                        Ok(())
                    } else {
                        Err("repository not found".into())
                    }
                }
            }
            "receive-pack" => {
                if is_owner {
                    Ok(())
                } else {
                    let role = db::collaborator::get_role(
                        &self.state.pool,
                        &repo.id.to_string(),
                        user_id,
                    )
                    .await
                    .unwrap_or(None);
                    match role {
                        Some(r) if r.has(CollaboratorRole::Write) => Ok(()),
                        _ => Err("permission denied: no push access".into()),
                    }
                }
            }
            _ => Err(format!("unsupported service: {}", service)),
        }
    }
}

/// Parse "git-upload-pack '/owner/repo.git'" → ("upload-pack", "/owner/repo.git")
fn parse_git_command(command: &str) -> Option<(String, String)> {
    let parts: Vec<&str> = command.splitn(2, ' ').collect();
    if parts.len() != 2 {
        return None;
    }

    let service = parts[0];
    if service != "git-upload-pack" && service != "git-receive-pack" {
        return None;
    }

    let path = parts[1].trim_matches('\'').trim_matches('"').to_string();
    let service_name = service.strip_prefix("git-").unwrap_or(service);
    Some((service_name.to_string(), path))
}

/// Parse "/owner/repo.git" → (owner, repo_name)
fn parse_repo_path(path: &str) -> Option<(String, String)> {
    let path = path.strip_prefix('/').unwrap_or(path);
    let parts: Vec<&str> = path.splitn(2, '/').collect();
    if parts.len() != 2 {
        return None;
    }

    let owner = parts[0].to_string();
    let repo = parts[1]
        .strip_suffix(".git")
        .unwrap_or(parts[1])
        .to_string();

    if owner.is_empty() || repo.is_empty() {
        return None;
    }

    Some((owner, repo))
}

/// Start the SSH server on the configured port.
pub async fn start_ssh_server(
    config: &delta_core::config::SshConfig,
    pool: SqlitePool,
    repos_dir: PathBuf,
    host: &str,
) -> anyhow::Result<()> {
    let host_key = load_or_generate_host_key(config, &repos_dir)?;

    let russh_config = russh::server::Config {
        keys: vec![host_key],
        auth_rejection_time: std::time::Duration::from_secs(1),
        auth_rejection_time_initial: Some(std::time::Duration::from_secs(0)),
        ..Default::default()
    };

    let state = SshServerState { pool, repos_dir };
    let mut server = DeltaSshServer { state };
    let addr = format!("{}:{}", host, config.port);
    tracing::info!("SSH server listening on {}", addr);

    server
        .run_on_address(Arc::new(russh_config), &addr)
        .await?;

    Ok(())
}

/// Load the host key from file, or generate a new ed25519 key.
fn load_or_generate_host_key(
    config: &delta_core::config::SshConfig,
    repos_dir: &Path,
) -> anyhow::Result<PrivateKey> {
    let key_path = if let Some(ref path) = config.host_key_file {
        PathBuf::from(path)
    } else {
        repos_dir
            .parent()
            .unwrap_or(repos_dir)
            .join("ssh_host_ed25519_key")
    };

    if key_path.exists() {
        tracing::info!(path = %key_path.display(), "loading SSH host key");
        let key = russh_keys::load_secret_key(&key_path, None)?;
        return Ok(key);
    }

    // Generate new ed25519 key
    tracing::info!(path = %key_path.display(), "generating new SSH host key");
    let key = PrivateKey::random(
        &mut russh_keys::ssh_key::rand_core::OsRng,
        russh_keys::Algorithm::Ed25519,
    )
    .map_err(|e| anyhow::anyhow!("failed to generate SSH host key: {}", e))?;

    // Ensure parent directory exists
    if let Some(parent) = key_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Write private key in OpenSSH format
    let encoded = key
        .to_openssh(russh_keys::ssh_key::LineEnding::LF)
        .map_err(|e| anyhow::anyhow!("failed to encode host key: {}", e))?;
    std::fs::write(&key_path, encoded.as_bytes())?;

    // Set restrictive permissions on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600))?;
    }

    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_git_command_upload_pack() {
        let (service, path) =
            parse_git_command("git-upload-pack '/alice/myrepo.git'").unwrap();
        assert_eq!(service, "upload-pack");
        assert_eq!(path, "/alice/myrepo.git");
    }

    #[test]
    fn test_parse_git_command_receive_pack() {
        let (service, path) =
            parse_git_command("git-receive-pack '/alice/myrepo.git'").unwrap();
        assert_eq!(service, "receive-pack");
        assert_eq!(path, "/alice/myrepo.git");
    }

    #[test]
    fn test_parse_git_command_invalid() {
        assert!(parse_git_command("ls -la").is_none());
        assert!(parse_git_command("git-evil '/foo'").is_none());
        assert!(parse_git_command("").is_none());
    }

    #[test]
    fn test_parse_repo_path() {
        let (owner, repo) = parse_repo_path("/alice/myrepo.git").unwrap();
        assert_eq!(owner, "alice");
        assert_eq!(repo, "myrepo");
    }

    #[test]
    fn test_parse_repo_path_no_git_suffix() {
        let (owner, repo) = parse_repo_path("/alice/myrepo").unwrap();
        assert_eq!(owner, "alice");
        assert_eq!(repo, "myrepo");
    }

    #[test]
    fn test_parse_repo_path_no_leading_slash() {
        let (owner, repo) = parse_repo_path("alice/myrepo.git").unwrap();
        assert_eq!(owner, "alice");
        assert_eq!(repo, "myrepo");
    }

    #[test]
    fn test_parse_repo_path_invalid() {
        assert!(parse_repo_path("myrepo.git").is_none());
        assert!(parse_repo_path("/").is_none());
        assert!(parse_repo_path("").is_none());
    }
}
