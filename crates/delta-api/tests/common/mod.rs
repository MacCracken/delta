//! Shared helpers for delta-api integration tests.

#![allow(dead_code)]

use delta_api::{routes, state::AppState};
use delta_core::DeltaConfig;

pub struct Server {
    pub base: String,
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
    let app = routes::router(AppState::new(config, pool));
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
        _tmp: tmp,
    }
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
