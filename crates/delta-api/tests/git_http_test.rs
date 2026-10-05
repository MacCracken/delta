//! End-to-end tests of the git smart HTTP transport using a real git client.

use std::path::Path;
use std::process::{Command, Output};

use delta_api::{routes, state::AppState};
use delta_core::DeltaConfig;

struct Server {
    base: String,
    _tmp: tempfile::TempDir,
}

/// Serve the full router on a random local port.
async fn start_server() -> Server {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = DeltaConfig::default();
    config.storage.repos_dir = tmp.path().join("repos");
    config.storage.artifacts_dir = tmp.path().join("artifacts");
    config.storage.db_url = format!(
        "sqlite://{}?mode=rwc",
        tmp.path().join("delta.db").display()
    );
    config.rate_limit.enabled = false;
    std::fs::create_dir_all(&config.storage.repos_dir).unwrap();
    std::fs::create_dir_all(&config.storage.artifacts_dir).unwrap();

    let pool = delta_core::db::init_pool(&config.storage.db_url)
        .await
        .unwrap();
    let app = routes::router(AppState::new(config, pool));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    Server {
        base: format!("http://{addr}"),
        _tmp: tmp,
    }
}

/// Register a user and create a repository; returns the user's API token.
async fn create_user_and_repo(server: &Server, user: &str, repo: &str, visibility: &str) -> String {
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

/// Run git without prompting for credentials.
async fn git(dir: &Path, args: &[&str]) -> Output {
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

async fn git_ok(dir: &Path, args: &[&str]) {
    let out = git(dir, args).await;
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[tokio::test]
async fn push_and_clone_over_http() {
    let server = start_server().await;
    let token = create_user_and_repo(&server, "alice", "demo", "private").await;
    let host = server.base.trim_start_matches("http://");
    let authed = format!("http://alice:{token}@{host}/alice/demo.git");

    let work = tempfile::tempdir().unwrap();
    git_ok(work.path(), &["init", "-q", "-b", "main"]).await;
    // Larger than axum's default 2 MiB request body limit, and incompressible.
    let data: Vec<u8> = (0..3 * 1024 * 1024u32)
        .map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8)
        .collect();
    std::fs::write(work.path().join("blob.bin"), &data).unwrap();
    git_ok(work.path(), &["add", "."]).await;
    git_ok(work.path(), &["commit", "-q", "-m", "init"]).await;
    // Credentials are only sent after a 401 carrying a WWW-Authenticate challenge.
    git_ok(work.path(), &["push", "-q", &authed, "main"]).await;

    let clone = tempfile::tempdir().unwrap();
    git_ok(clone.path(), &["clone", "-q", &authed, "."]).await;
    assert_eq!(std::fs::read(clone.path().join("blob.bin")).unwrap(), data);

    // A private repository is not readable anonymously.
    let anon = format!("{}/alice/demo.git", server.base);
    let out = git(clone.path(), &["ls-remote", &anon]).await;
    assert!(!out.status.success());
}

#[tokio::test]
async fn branch_protection_is_enforced_on_push() {
    let server = start_server().await;
    let token = create_user_and_repo(&server, "bob", "prot", "public").await;
    let host = server.base.trim_start_matches("http://");
    let url = format!("http://bob:{token}@{host}/bob/prot.git");

    let work = tempfile::tempdir().unwrap();
    let dir = work.path();
    git_ok(dir, &["init", "-q", "-b", "main"]).await;
    git_ok(dir, &["commit", "-q", "--allow-empty", "-m", "one"]).await;
    git_ok(dir, &["commit", "-q", "--allow-empty", "-m", "two"]).await;
    git_ok(dir, &["push", "-q", &url, "main"]).await;

    let client = reqwest::Client::new();
    let status = client
        .post(format!(
            "{}/api/v1/repos/bob/prot/branch-protections",
            server.base
        ))
        .bearer_auth(&token)
        .json(&serde_json::json!({
            "pattern": "main",
            "prevent_force_push": true,
            "prevent_deletion": true,
        }))
        .send()
        .await
        .unwrap()
        .status();
    assert!(status.is_success());

    // Rewriting history is refused; fast-forwards still work.
    git_ok(dir, &["reset", "-q", "--hard", "HEAD~1"]).await;
    git_ok(dir, &["commit", "-q", "--allow-empty", "-m", "rewritten"]).await;
    assert!(
        !git(dir, &["push", "-f", &url, "main"])
            .await
            .status
            .success()
    );
    git_ok(dir, &["pull", "-q", "--rebase", &url, "main"]).await;
    git_ok(dir, &["push", "-q", &url, "main"]).await;

    let out = git(dir, &["push", &url, ":main"]).await;
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("deletion is not allowed"));

    // Any matching rule requiring pull requests blocks direct pushes.
    let status = client
        .post(format!(
            "{}/api/v1/repos/bob/prot/branch-protections",
            server.base
        ))
        .bearer_auth(&token)
        .json(&serde_json::json!({ "pattern": "m*", "require_pr": true }))
        .send()
        .await
        .unwrap()
        .status();
    assert!(status.is_success());
    git_ok(dir, &["commit", "-q", "--allow-empty", "-m", "direct"]).await;
    let out = git(dir, &["push", &url, "main"]).await;
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("pull request"));

    // Unprotected branches are unaffected.
    git_ok(dir, &["push", "-q", &url, "HEAD:feature/x"]).await;
}
