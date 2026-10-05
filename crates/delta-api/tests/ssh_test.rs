//! End-to-end tests of the SSH git transport using real git and OpenSSH
//! clients. Skipped when no `ssh` client is installed.

mod common;

use std::path::Path;
use std::process::Output;
use std::time::Duration;

use common::{Server, create_user_and_repo, git_ok, git_with_env, post, register, start_server};
use serde_json::json;

fn ssh_available() -> bool {
    let found = std::process::Command::new("ssh")
        .arg("-V")
        .output()
        .is_ok_and(|o| o.status.success());
    if !found {
        eprintln!("skipping: no ssh client installed");
    }
    found
}

/// Serve SSH for `server` on a random port; returns the port.
async fn start_ssh(server: &Server) -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let state = server.state.clone();
    tokio::spawn(async move { delta_api::ssh::serve_ssh(state, listener).await.unwrap() });
    port
}

/// An SSH identity registered for a user.
struct Identity {
    _dir: tempfile::TempDir,
    /// `GIT_SSH_COMMAND` that authenticates with this identity.
    ssh_command: String,
}

impl Identity {
    async fn new(server: &Server, token: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let key = dir.path().join("id_ed25519");
        let out = std::process::Command::new("ssh-keygen")
            .args(["-q", "-t", "ed25519", "-N", "", "-f"])
            .arg(&key)
            .output()
            .unwrap();
        assert!(out.status.success(), "ssh-keygen failed");
        let public_key = std::fs::read_to_string(dir.path().join("id_ed25519.pub")).unwrap();
        let (status, body) = post(
            server,
            token,
            "/user/ssh-keys",
            json!({ "name": "laptop", "public_key": public_key }),
        )
        .await;
        assert!(status.is_success(), "add key: {status} {body}");
        let ssh_command = format!(
            "ssh -F /dev/null -i {} -o IdentitiesOnly=yes -o BatchMode=yes \
             -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR",
            key.display()
        );
        Self {
            _dir: dir,
            ssh_command,
        }
    }

    async fn git(&self, dir: &Path, args: &[&str]) -> Output {
        git_with_env(dir, args, &[("GIT_SSH_COMMAND", &self.ssh_command)]).await
    }

    async fn git_ok(&self, dir: &Path, args: &[&str]) {
        let out = self.git(dir, args).await;
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

fn head(dir: &Path) -> String {
    let out = std::process::Command::new("git")
        .current_dir(dir)
        .args(["rev-parse", "HEAD"])
        .output()
        .unwrap();
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

#[tokio::test]
async fn push_clone_and_fetch_over_ssh() {
    if !ssh_available() {
        return;
    }
    let server = start_server().await;
    let token = create_user_and_repo(&server, "alice", "demo", "private").await;
    let alice = Identity::new(&server, &token).await;
    let port = start_ssh(&server).await;
    let url = format!("ssh://git@127.0.0.1:{port}/alice/demo.git");

    let work = tempfile::tempdir().unwrap();
    let dir = work.path();
    git_ok(dir, &["init", "-q", "-b", "main"]).await;
    // Larger than any single pipe or channel buffer.
    let data = common::incompressible(3 * 1024 * 1024);
    std::fs::write(dir.join("blob.bin"), &data).unwrap();
    std::fs::create_dir_all(dir.join(".delta/workflows")).unwrap();
    std::fs::write(
        dir.join(".delta/workflows/ci.toml"),
        "name = \"CI\"\n\n[[on]]\npush = { branches = [\"main\"] }\n\n\
         [jobs.build]\nneeds = []\n\n[[jobs.build.steps]]\nname = \"noop\"\nrun = \"true\"\n",
    )
    .unwrap();
    git_ok(dir, &["add", "."]).await;
    git_ok(dir, &["commit", "-q", "-m", "init"]).await;
    alice.git_ok(dir, &["push", "-q", &url, "main"]).await;

    // Clone with protocol v2 (git's default) and v0.
    for (version, target) in [("2", "v2"), ("0", "v0")] {
        let clone = work.path().join(target);
        let protocol = format!("protocol.version={version}");
        alice
            .git_ok(
                work.path(),
                &[
                    "-c",
                    &protocol,
                    "clone",
                    "-q",
                    &url,
                    clone.to_str().unwrap(),
                ],
            )
            .await;
        assert_eq!(std::fs::read(clone.join("blob.bin")).unwrap(), data);
    }

    // Incremental fetch negotiates against what the clone already has.
    std::fs::write(dir.join("more.txt"), "more").unwrap();
    git_ok(dir, &["add", "."]).await;
    git_ok(dir, &["commit", "-q", "-m", "more"]).await;
    alice.git_ok(dir, &["push", "-q", &url, "main"]).await;
    let clone = work.path().join("v2");
    alice.git_ok(&clone, &["pull", "-q", "--ff-only"]).await;
    assert_eq!(head(&clone), head(dir));

    // Pushes over SSH fire the same events as over HTTP.
    let mut pipelines = serde_json::Value::Null;
    for _ in 0..50 {
        pipelines = reqwest::Client::new()
            .get(format!("{}/api/v1/repos/alice/demo/pipelines", server.base))
            .bearer_auth(&token)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        if pipelines.as_array().is_some_and(|p| p.len() >= 2) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(pipelines.as_array().map(Vec::len), Some(2), "{pipelines}");

    // Users without access see nothing, and can't push.
    let bob_token = register(&server, "bob").await;
    let bob = Identity::new(&server, &bob_token).await;
    let out = bob
        .git(
            work.path(),
            &[
                "clone",
                "-q",
                &url,
                work.path().join("bob").to_str().unwrap(),
            ],
        )
        .await;
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("repository not found"));
    let out = bob.git(dir, &["push", &url, "HEAD:refs/heads/bob"]).await;
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("repository not found"));
}

#[tokio::test]
async fn ssh_push_respects_branch_protection() {
    if !ssh_available() {
        return;
    }
    let server = start_server().await;
    let token = create_user_and_repo(&server, "bob", "prot", "public").await;
    let bob = Identity::new(&server, &token).await;
    let port = start_ssh(&server).await;
    let url = format!("ssh://git@127.0.0.1:{port}/bob/prot.git");

    let work = tempfile::tempdir().unwrap();
    let dir = work.path();
    git_ok(dir, &["init", "-q", "-b", "main"]).await;
    git_ok(dir, &["commit", "-q", "--allow-empty", "-m", "one"]).await;
    git_ok(dir, &["commit", "-q", "--allow-empty", "-m", "two"]).await;
    bob.git_ok(dir, &["push", "-q", &url, "main"]).await;

    let (status, _) = post(
        &server,
        &token,
        "/repos/bob/prot/branch-protections",
        json!({ "pattern": "main", "prevent_force_push": true, "prevent_deletion": true }),
    )
    .await;
    assert!(status.is_success());

    // Rewriting history is refused; fast-forwards still work.
    git_ok(dir, &["reset", "-q", "--hard", "HEAD~1"]).await;
    git_ok(dir, &["commit", "-q", "--allow-empty", "-m", "rewritten"]).await;
    let out = bob.git(dir, &["push", "-f", &url, "main"]).await;
    assert!(!out.status.success());
    bob.git_ok(dir, &["pull", "-q", "--rebase", &url, "main"])
        .await;
    bob.git_ok(dir, &["push", "-q", &url, "main"]).await;

    let out = bob.git(dir, &["push", &url, ":main"]).await;
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("deletion is not allowed"));

    let (status, _) = post(
        &server,
        &token,
        "/repos/bob/prot/branch-protections",
        json!({ "pattern": "m*", "require_pr": true }),
    )
    .await;
    assert!(status.is_success());
    // Rejected before git reads the pack: the client still gets the reason
    // while it is sending a large one.
    std::fs::write(dir.join("big.bin"), common::incompressible(3 * 1024 * 1024)).unwrap();
    git_ok(dir, &["add", "big.bin"]).await;
    git_ok(dir, &["commit", "-q", "-m", "direct"]).await;
    let out = bob.git(dir, &["push", &url, "main"]).await;
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("pull request"));

    // Unprotected branches are unaffected, and other users can fetch a
    // public repository but not push to it.
    bob.git_ok(dir, &["push", "-q", &url, "HEAD:feature/x"])
        .await;
    let carol_token = register(&server, "carol").await;
    let carol = Identity::new(&server, &carol_token).await;
    carol.git_ok(dir, &["fetch", "-q", &url, "feature/x"]).await;
    let out = carol.git(dir, &["push", &url, "HEAD:feature/y"]).await;
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("push access"));
}
