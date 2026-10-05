//! End-to-end tests of the git smart HTTP transport using a real git client.

mod common;

use common::{create_user_and_repo, git, git_ok, post, register, start_server};

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

#[tokio::test]
async fn merge_gate_checks_the_current_head() {
    let server = start_server().await;
    let owner = create_user_and_repo(&server, "olive", "gate", "public").await;
    let stranger = register(&server, "mallory").await;
    let reviewer = register(&server, "carol").await;
    let host = server.base.trim_start_matches("http://");
    let url = format!("http://olive:{owner}@{host}/olive/gate.git");

    let work = tempfile::tempdir().unwrap();
    let dir = work.path();
    git_ok(dir, &["init", "-q", "-b", "main"]).await;
    git_ok(dir, &["commit", "-q", "--allow-empty", "-m", "base"]).await;
    git_ok(dir, &["push", "-q", &url, "main"]).await;
    git_ok(dir, &["checkout", "-q", "-b", "feature"]).await;
    git_ok(dir, &["commit", "-q", "--allow-empty", "-m", "reviewed"]).await;
    git_ok(dir, &["push", "-q", &url, "feature"]).await;

    let (status, _) = post(
        &server,
        &owner,
        "/repos/olive/gate/collaborators",
        serde_json::json!({ "username": "carol", "role": "write" }),
    )
    .await;
    assert!(status.is_success());
    let (status, _) = post(&server, &owner, "/repos/olive/gate/branch-protections",
        serde_json::json!({ "pattern": "main", "required_approvals": 1, "require_status_checks": true })).await;
    assert!(status.is_success());
    let (status, pr) = post(
        &server,
        &owner,
        "/repos/olive/gate/pulls",
        serde_json::json!({ "title": "feature", "head_branch": "feature", "base_branch": "main" }),
    )
    .await;
    assert!(status.is_success(), "create PR: {status} {pr}");
    let number = pr["number"].as_i64().unwrap();
    let merge = format!("/repos/olive/gate/pulls/{number}/merge");
    let reviews = format!("/repos/olive/gate/pulls/{number}/reviews");
    let approve = serde_json::json!({ "state": "approved" });

    // An approval from someone without write access doesn't count.
    let (status, _) = post(&server, &stranger, &reviews, approve.clone()).await;
    assert!(status.is_success());
    let (status, _) = post(&server, &owner, &merge, serde_json::json!({})).await;
    assert_eq!(status, reqwest::StatusCode::CONFLICT);

    // A valid approval, but no status checks reported yet.
    let (status, _) = post(&server, &reviewer, &reviews, approve.clone()).await;
    assert!(status.is_success());
    let (status, _) = post(&server, &owner, &merge, serde_json::json!({})).await;
    assert_eq!(status, reqwest::StatusCode::CONFLICT);

    let reviewed_sha = String::from_utf8(git(dir, &["rev-parse", "HEAD"]).await.stdout).unwrap();
    let success = serde_json::json!({ "context": "ci", "state": "success" });
    let (status, _) = post(
        &server,
        &owner,
        &format!("/repos/olive/gate/commits/{}/statuses", reviewed_sha.trim()),
        success.clone(),
    )
    .await;
    assert!(status.is_success());

    // New, unreviewed commits invalidate the approval and the checks.
    git_ok(dir, &["commit", "-q", "--allow-empty", "-m", "unreviewed"]).await;
    git_ok(dir, &["push", "-q", &url, "feature"]).await;
    let (status, body) = post(&server, &owner, &merge, serde_json::json!({})).await;
    assert_eq!(status, reqwest::StatusCode::CONFLICT, "{body}");

    // Approving and checking the new head allows the merge.
    let head_sha = String::from_utf8(git(dir, &["rev-parse", "HEAD"]).await.stdout).unwrap();
    let (status, _) = post(&server, &reviewer, &reviews, approve).await;
    assert!(status.is_success());
    let (status, _) = post(
        &server,
        &owner,
        &format!("/repos/olive/gate/commits/{}/statuses", head_sha.trim()),
        success,
    )
    .await;
    assert!(status.is_success());
    let (status, body) = post(&server, &owner, &merge, serde_json::json!({})).await;
    assert!(status.is_success(), "merge: {status} {body}");
}
