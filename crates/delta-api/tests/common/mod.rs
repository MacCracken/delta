//! Shared helpers for delta-api integration tests.

#![allow(dead_code)]

use std::path::Path;
use std::process::{Command, Output};

use delta_api::{routes, state::AppState};
use delta_core::DeltaConfig;

pub struct Server {
    pub base: String,
    /// The state the server runs with (database, repository storage, ...).
    pub state: AppState,
    _tmp: tempfile::TempDir,
}

/// Serve the full router on a random local port.
pub async fn start_server() -> Server {
    start_server_with(|config| config.rate_limit.enabled = false).await
}

/// Like [`start_server`], with a chance to adjust the configuration.
pub async fn start_server_with(configure: impl FnOnce(&mut DeltaConfig)) -> Server {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = DeltaConfig::default();
    config.storage.repos_dir = tmp.path().join("repos");
    config.storage.artifacts_dir = tmp.path().join("artifacts");
    config.storage.db_url = format!(
        "sqlite://{}?mode=rwc",
        tmp.path().join("delta.db").display()
    );
    std::fs::create_dir_all(&config.storage.repos_dir).unwrap();
    std::fs::create_dir_all(&config.storage.artifacts_dir).unwrap();

    configure(&mut config);
    let pool = delta_core::db::init_pool(&config.storage.db_url)
        .await
        .unwrap();
    let state = AppState::new(config, pool);
    let app = routes::router(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .unwrap();
    });
    Server {
        base: format!("http://{addr}"),
        state,
        _tmp: tmp,
    }
}

/// Run git without prompting for credentials or reading user configuration.
pub async fn git(dir: &Path, args: &[&str]) -> Output {
    let dir = dir.to_path_buf();
    let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    tokio::task::spawn_blocking(move || {
        Command::new("git")
            .current_dir(dir)
            .args(&args)
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_AUTHOR_NAME", "Test")
            .env("GIT_AUTHOR_EMAIL", "test@example.com")
            .env("GIT_COMMITTER_NAME", "Test")
            .env("GIT_COMMITTER_EMAIL", "test@example.com")
            .output()
            .unwrap()
    })
    .await
    .unwrap()
}

pub async fn git_ok(dir: &Path, args: &[&str]) {
    let out = git(dir, args).await;
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// POST JSON as `token`; returns the response status and body.
pub async fn post(
    server: &Server,
    token: &str,
    path: &str,
    body: serde_json::Value,
) -> (reqwest::StatusCode, serde_json::Value) {
    let res = reqwest::Client::new()
        .post(format!("{}/api/v1{}", server.base, path))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .unwrap();
    let status = res.status();
    let body = res.json().await.unwrap_or(serde_json::Value::Null);
    (status, body)
}

/// Register a user; returns their API token.
pub async fn register(server: &Server, user: &str) -> String {
    let res: serde_json::Value = reqwest::Client::new()
        .post(format!("{}/api/v1/auth/register", server.base))
        .json(&serde_json::json!({
            "username": user,
            "email": format!("{user}@example.com"),
            "password": "correct horse battery staple",
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    res["token"].as_str().unwrap().to_string()
}

/// Register a user and create a repository; returns the user's API token.
pub async fn create_user_and_repo(
    server: &Server,
    user: &str,
    repo: &str,
    visibility: &str,
) -> String {
    let client = reqwest::Client::new();
    let res: serde_json::Value = client
        .post(format!("{}/api/v1/auth/register", server.base))
        .json(&serde_json::json!({
            "username": user,
            "email": format!("{user}@example.com"),
            "password": "correct horse battery staple",
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let token = res["token"].as_str().unwrap().to_string();
    let status = client
        .post(format!("{}/api/v1/repos", server.base))
        .bearer_auth(&token)
        .json(&serde_json::json!({ "name": repo, "visibility": visibility }))
        .send()
        .await
        .unwrap()
        .status();
    assert!(status.is_success(), "create repo: {status}");
    token
}
