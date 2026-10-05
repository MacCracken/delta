//! Built-in SSH server for git transport.
//!
//! Accepts SSH connections, authenticates users by public key, and serves
//! `git-upload-pack` (clone/fetch) and `git-receive-pack` (push) over the
//! session channel with the same access rules, branch protection and push
//! events as the HTTP transport.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use delta_core::models::collaborator::CollaboratorRole;
use delta_core::models::repo::{Repository, Visibility};
use delta_core::models::user::User;
use russh::keys::{self as russh_keys, HashAlg, PrivateKey, PublicKey};
use russh::server::{Auth, ChannelOpenHandle, Handler, Msg, Server, Session};
use russh::{Channel, ChannelId, ChannelOpenFailure};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::state::AppState;

/// Session channels (and so git processes) one connection may have open.
const MAX_CHANNELS_PER_CONNECTION: usize = 4;
/// Upper bound on one git command.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(60 * 60);

/// The SSH server — creates a new handler per connection.
pub struct DeltaSshServer {
    state: AppState,
}

impl Server for DeltaSshServer {
    type Handler = SshSession;

    fn new_client(&mut self, _peer_addr: Option<std::net::SocketAddr>) -> Self::Handler {
        SshSession {
            state: self.state.clone(),
            user: None,
            channels: HashMap::new(),
            slots: Arc::new(Semaphore::new(MAX_CHANNELS_PER_CONNECTION)),
        }
    }
}

/// Per-connection session state.
pub struct SshSession {
    state: AppState,
    /// The authenticated user.
    user: Option<User>,
    /// Session channels that have not started a command yet.
    channels: HashMap<ChannelId, PendingChannel>,
    /// Bounds the channels open on this connection.
    slots: Arc<Semaphore>,
}

struct PendingChannel {
    channel: Channel<Msg>,
    /// The client's `GIT_PROTOCOL` request (git sends it with `SendEnv`).
    git_protocol: Option<String>,
    _slot: OwnedSemaphorePermit,
}

impl Handler for SshSession {
    type Error = anyhow::Error;

    async fn auth_publickey(
        &mut self,
        _user: &str,
        public_key: &PublicKey,
    ) -> Result<Auth, Self::Error> {
        let fingerprint = public_key.fingerprint(HashAlg::Sha256).to_string();
        let user =
            match delta_core::db::ssh_key::get_user_by_fingerprint(&self.state.db, &fingerprint)
                .await
            {
                Ok(Some((user_id, _))) => delta_core::db::user::get_by_id(&self.state.db, &user_id)
                    .await
                    .ok(),
                _ => None,
            };
        match user {
            Some(user) => {
                tracing::info!(username = %user.username, "SSH auth success");
                self.user = Some(user);
                Ok(Auth::Accept)
            }
            None => {
                tracing::debug!(fingerprint = %fingerprint, "SSH auth failed: key not found");
                Ok(Auth::reject())
            }
        }
    }

    async fn auth_password(&mut self, _user: &str, _password: &str) -> Result<Auth, Self::Error> {
        Ok(Auth::reject())
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<Msg>,
        reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        if self.user.is_none() {
            reply
                .reject(ChannelOpenFailure::AdministrativelyProhibited)
                .await;
            return Ok(());
        }
        let Ok(slot) = self.slots.clone().try_acquire_owned() else {
            reply.reject(ChannelOpenFailure::ResourceShortage).await;
            return Ok(());
        };
        self.channels.insert(
            channel.id(),
            PendingChannel {
                channel,
                git_protocol: None,
                _slot: slot,
            },
        );
        reply.accept().await;
        Ok(())
    }

    async fn env_request(
        &mut self,
        channel_id: ChannelId,
        variable_name: &str,
        variable_value: &str,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        if variable_name == "GIT_PROTOCOL"
            && is_valid_git_protocol(variable_value)
            && let Some(pending) = self.channels.get_mut(&channel_id)
        {
            pending.git_protocol = Some(variable_value.to_string());
        }
        Ok(())
    }

    async fn exec_request(
        &mut self,
        channel_id: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let (Some(pending), Some(user)) = (self.channels.remove(&channel_id), self.user.clone())
        else {
            session.channel_failure(channel_id)?;
            return Ok(());
        };
        session.channel_success(channel_id)?;
        let command = String::from_utf8_lossy(data).into_owned();
        tracing::debug!(command = %command, "SSH exec request");
        tokio::spawn(serve_channel(self.state.clone(), user, command, pending));
        Ok(())
    }

    async fn shell_request(
        &mut self,
        channel_id: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let (Some(_), Some(user)) = (self.channels.remove(&channel_id), &self.user) else {
            session.channel_failure(channel_id)?;
            return Ok(());
        };
        session.channel_success(channel_id)?;
        let greeting = format!(
            "Hi {}! You've successfully authenticated, but Delta does not provide shell access.\r\n",
            user.username
        );
        session.extended_data(channel_id, 1, greeting.into_bytes())?;
        session.exit_status_request(channel_id, 1)?;
        session.eof(channel_id)?;
        session.close(channel_id)?;
        Ok(())
    }

    async fn subsystem_request(
        &mut self,
        channel_id: ChannelId,
        _name: &str,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        session.channel_failure(channel_id)?;
        Ok(())
    }

    async fn channel_close(
        &mut self,
        channel_id: ChannelId,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.channels.remove(&channel_id);
        Ok(())
    }
}

/// Run the command `user` sent on a channel and report how it went.
async fn serve_channel(state: AppState, user: User, command: String, pending: PendingChannel) {
    let PendingChannel {
        channel,
        git_protocol,
        _slot,
    } = pending;
    let (mut read_half, write_half) = channel.split();
    let mut input = read_half.make_reader();
    let mut output = write_half.make_writer();

    let result = tokio::time::timeout(
        COMMAND_TIMEOUT,
        run_command(
            &state,
            &user,
            &command,
            git_protocol.as_deref(),
            &mut input,
            &mut output,
        ),
    )
    .await
    .unwrap_or_else(|_| Err("timed out".into()));

    let status = match result {
        Ok(()) => 0,
        Err(message) => {
            let mut stderr = write_half.make_writer_ext(Some(1));
            let _ = stderr
                .write_all(format!("ERROR: {message}\n").as_bytes())
                .await;
            1
        }
    };
    // Close without waiting for the client's EOF, as sshd does once the
    // command exits: OpenSSH clients only release their output (git's input)
    // when the channel closes, and git holds its side open until then.
    let _ = write_half.exit_status(status).await;
    let _ = write_half.eof().await;
    let _ = write_half.close().await;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GitService {
    UploadPack,
    ReceivePack,
}

/// Run one git command; `Err` carries the message for the client.
async fn run_command<R, W>(
    state: &AppState,
    user: &User,
    command: &str,
    git_protocol: Option<&str>,
    input: &mut R,
    output: &mut W,
) -> Result<(), String>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let (service, path) = parse_git_command(command)
        .ok_or("unsupported command: only git clone, fetch and push are available")?;
    let (owner, name) = parse_repo_path(&path).ok_or("invalid repository path")?;
    let repo_path = state
        .repo_host
        .repo_path(&owner, &name)
        .map_err(|_| "invalid repository path")?;
    let repo = authorize(state, user, service, &owner, &name).await?;
    if !repo_path.exists() {
        return Err("repository not found".into());
    }

    match service {
        GitService::UploadPack => upload_pack(&repo_path, git_protocol, input, output).await,
        GitService::ReceivePack => {
            // Branch protection needs the client's commands before git
            // runs, so the push is split like a smart HTTP push: the ref
            // advertisement first, then the commands and pack.
            let advertisement =
                delta_vcs::protocol::ref_advertisement(&repo_path, "git-receive-pack")
                    .await
                    .map_err(|e| {
                        tracing::error!("receive-pack advertisement failed: {}", e);
                        "internal error".to_string()
                    })?;
            output
                .write_all(&advertisement)
                .await
                .map_err(|e| e.to_string())?;
            crate::routes::git::serve_push(state, &owner, &repo, &repo_path, user, input, output)
                .await
                .map_err(|(_, message)| message)
        }
    }
}

/// Serve a clone or fetch: git talks to the client directly.
async fn upload_pack<R, W>(
    repo_path: &Path,
    git_protocol: Option<&str>,
    input: &mut R,
    output: &mut W,
) -> Result<(), String>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let internal = |e: &dyn std::fmt::Display| {
        tracing::error!("SSH upload-pack failed: {}", e);
        "internal error".to_string()
    };
    let mut child =
        delta_vcs::protocol::spawn_service_session(repo_path, "upload-pack", git_protocol)
            .map_err(|e| internal(&e))?;
    let (Some(mut stdin), Some(mut stdout)) = (child.stdin.take(), child.stdout.take()) else {
        return Err(internal(&"git pipes unavailable"));
    };

    // git reads requests while it writes responses, so both directions
    // stream concurrently. Done when git is: the client may wait for the
    // channel to close before it closes its own side.
    let feed = async move {
        let _ = tokio::io::copy(input, &mut stdin).await;
        // Closing (not just shutting down) the pipe is what git sees as EOF.
        drop(stdin);
    };
    let respond = tokio::io::copy(&mut stdout, output);
    tokio::pin!(feed, respond);
    let responded = tokio::select! {
        responded = &mut respond => responded,
        () = &mut feed => respond.await,
    };
    if let Err(e) = responded {
        // Typically the client went away.
        tracing::debug!("SSH upload-pack stream ended: {}", e);
        return Err("fetch interrupted".into());
    }
    let status = child.wait().await.map_err(|e| internal(&e))?;
    if status.success() {
        Ok(())
    } else {
        // git has already told the client what went wrong.
        tracing::debug!(%status, "git upload-pack exited with error");
        Err("fetch failed".into())
    }
}

/// Check `user` may run `service` on `owner/name`. Repositories the user
/// can't read are reported as missing.
async fn authorize(
    state: &AppState,
    user: &User,
    service: GitService,
    owner: &str,
    name: &str,
) -> Result<Repository, String> {
    let repo = crate::routes::git::find_repo(state, owner, name)
        .await
        .map_err(|(_, message)| message)?;
    if repo.owner == user.id.to_string() {
        return Ok(repo);
    }
    let role = delta_core::db::collaborator::get_role(
        &state.db,
        &repo.id.to_string(),
        &user.id.to_string(),
    )
    .await
    .unwrap_or(None);
    let can_read = repo.visibility == Visibility::Public || role.is_some();
    match service {
        GitService::UploadPack if can_read => Ok(repo),
        GitService::ReceivePack if role.is_some_and(|r| r.has(CollaboratorRole::Write)) => Ok(repo),
        GitService::ReceivePack if can_read => {
            Err("you don't have push access to this repository".into())
        }
        _ => Err("repository not found".into()),
    }
}

/// `GIT_PROTOCOL` values passed on to git, e.g. `version=2`.
fn is_valid_git_protocol(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"=:._-".contains(&b))
}

/// Parse "git-upload-pack '/owner/repo.git'" → (UploadPack, "/owner/repo.git")
fn parse_git_command(command: &str) -> Option<(GitService, String)> {
    let (service, path) = command.split_once(' ')?;
    let service = match service {
        "git-upload-pack" => GitService::UploadPack,
        "git-receive-pack" => GitService::ReceivePack,
        _ => return None,
    };
    let path = path.trim_matches('\'').trim_matches('"').to_string();
    Some((service, path))
}

/// Parse "/owner/repo.git" → (owner, repo_name)
fn parse_repo_path(path: &str) -> Option<(String, String)> {
    let path = path.strip_prefix('/').unwrap_or(path);
    let (owner, repo) = path.split_once('/')?;
    let repo = repo.strip_suffix(".git").unwrap_or(repo);
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    Some((owner.to_string(), repo.to_string()))
}

/// Start the SSH server on the configured port.
pub async fn start_ssh_server(state: AppState) -> anyhow::Result<()> {
    let addr = format!("{}:{}", state.config.server.host, state.config.ssh.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!("SSH server listening on {}", addr);
    serve_ssh(state, listener).await
}

/// Serve SSH connections accepted from `listener`.
pub async fn serve_ssh(state: AppState, listener: tokio::net::TcpListener) -> anyhow::Result<()> {
    let host_key = load_or_generate_host_key(&state.config.ssh, &state.config.storage.repos_dir)?;

    let russh_config = russh::server::Config {
        keys: vec![host_key],
        auth_rejection_time: Duration::from_secs(1),
        auth_rejection_time_initial: Some(Duration::from_secs(0)),
        ..Default::default()
    };

    let mut server = DeltaSshServer { state };
    server
        .run_on_socket(Arc::new(russh_config), &listener)
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
    let mut seed = [0u8; 32];
    getrandom::fill(&mut seed)
        .map_err(|e| anyhow::anyhow!("failed to generate SSH host key: {}", e))?;
    let key = PrivateKey::from(russh_keys::ssh_key::private::Ed25519Keypair::from_seed(
        &seed,
    ));

    // Ensure parent directory exists
    if let Some(parent) = key_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Write private key in OpenSSH format
    let encoded = key
        .to_openssh(russh_keys::ssh_key::LineEnding::LF)
        .map_err(|e| anyhow::anyhow!("failed to encode host key: {}", e))?;
    write_private_file(&key_path, encoded.as_bytes())?;

    Ok(key)
}

/// Create `path` holding `data`, readable only by its owner from the start
/// (never briefly world-readable), refusing to replace an existing file.
fn write_private_file(path: &Path, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(data)?;
    file.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_git_command_upload_pack() {
        let (service, path) = parse_git_command("git-upload-pack '/alice/myrepo.git'").unwrap();
        assert_eq!(service, GitService::UploadPack);
        assert_eq!(path, "/alice/myrepo.git");
    }

    #[test]
    fn test_parse_git_command_receive_pack() {
        let (service, path) = parse_git_command("git-receive-pack '/alice/myrepo.git'").unwrap();
        assert_eq!(service, GitService::ReceivePack);
        assert_eq!(path, "/alice/myrepo.git");
    }

    #[test]
    fn test_parse_git_command_invalid() {
        assert!(parse_git_command("ls -la").is_none());
        assert!(parse_git_command("git-evil '/foo'").is_none());
        assert!(parse_git_command("git-upload-archive '/alice/myrepo.git'").is_none());
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

    #[test]
    fn test_git_protocol_values() {
        assert!(is_valid_git_protocol("version=2"));
        assert!(is_valid_git_protocol("version=2:object-format=sha256"));
        assert!(!is_valid_git_protocol(""));
        assert!(!is_valid_git_protocol("version=2\nX=1"));
        assert!(!is_valid_git_protocol(&"v".repeat(200)));
    }

    #[test]
    fn test_host_key_file_is_private_and_never_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("key");
        write_private_file(&path, b"secret").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        assert!(write_private_file(&path, b"other").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"secret");
    }
}
