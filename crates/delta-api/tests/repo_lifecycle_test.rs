//! Integration tests for repository creation, forking, deletion and
//! workspace expiry.

mod common;

use common::{Server, create_user_and_repo, git, git_ok, post, register, start_server};
use reqwest::StatusCode;
use serde_json::json;

async fn get_status(server: &Server, path: &str) -> StatusCode {
    reqwest::get(format!("{}/api/v1{path}", server.base))
        .await
        .unwrap()
        .status()
}

#[tokio::test]
async fn forks_are_never_more_visible_than_their_source() {
    let server = start_server().await;
    let olga = create_user_and_repo(&server, "olga", "secret", "private").await;
    let (status, _) = post(
        &server,
        &olga,
        "/repos",
        json!({ "name": "team", "visibility": "internal" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, _) = post(
        &server,
        &olga,
        "/repos",
        json!({ "name": "open", "visibility": "public" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let bob = register(&server, "bob").await;
    for repo in ["secret", "team"] {
        let (status, _) = post(
            &server,
            &olga,
            &format!("/repos/olga/{repo}/collaborators"),
            json!({ "username": "bob", "role": "read" }),
        )
        .await;
        assert!(status.is_success(), "{status}");
    }

    for (source, expected) in [
        ("secret", "private"),
        ("team", "internal"),
        ("open", "public"),
    ] {
        let (status, fork) = post(
            &server,
            &bob,
            &format!("/repos/olga/{source}/forks"),
            json!({ "visibility": "public" }),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{fork}");
        assert_eq!(fork["visibility"], expected, "fork of {source}");
    }
    // The private source's contents are not exposed through the fork.
    assert_eq!(
        get_status(&server, "/repos/bob/secret").await,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn create_repo_does_not_adopt_existing_storage() {
    let server = start_server().await;
    let token = register(&server, "rita").await;
    // Data left on disk, e.g. by an earlier repository of the same name.
    let stale = server.state.repo_host.repo_path("rita", "notes").unwrap();
    std::fs::create_dir_all(&stale).unwrap();

    let (status, _) = post(&server, &token, "/repos", json!({ "name": "notes" })).await;
    assert_eq!(status, StatusCode::CONFLICT);
    // No orphaned record that would block the name or point at foreign data.
    let records: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM repositories WHERE name = 'notes'")
        .fetch_one(&server.state.db)
        .await
        .unwrap();
    assert_eq!(records, 0);

    std::fs::remove_dir_all(&stale).unwrap();
    let (status, _) = post(&server, &token, "/repos", json!({ "name": "notes" })).await;
    assert_eq!(status, StatusCode::CREATED);
}

#[tokio::test]
async fn delete_repo_purges_its_search_index() {
    let server = start_server().await;
    let token = create_user_and_repo(&server, "sam", "code", "private").await;
    let repo_id: String = sqlx::query_scalar("SELECT id FROM repositories WHERE name = 'code'")
        .fetch_one(&server.state.db)
        .await
        .unwrap();
    delta_core::db::search::index_file(&server.state.db, &repo_id, "key.txt", "hunter2")
        .await
        .unwrap();

    let status = reqwest::Client::new()
        .delete(format!("{}/api/v1/repos/sam/code", server.base))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .status();
    assert!(status.is_success(), "{status}");
    let hits = delta_core::db::search::search_repo(&server.state.db, &repo_id, "hunter2", 10)
        .await
        .unwrap();
    assert!(hits.is_empty(), "deleted repository still indexed");
}

#[tokio::test]
async fn expired_workspaces_lose_their_branches() {
    let server = start_server().await;
    let token = create_user_and_repo(&server, "wendy", "proj", "public").await;
    let bare = server.state.repo_host.repo_path("wendy", "proj").unwrap();
    let work = tempfile::tempdir().unwrap();
    git_ok(work.path(), &["init", "-q", "-b", "main"]).await;
    git_ok(
        work.path(),
        &["commit", "-q", "--allow-empty", "-m", "init"],
    )
    .await;
    git_ok(work.path(), &["push", "-q", bare.to_str().unwrap(), "main"]).await;

    let (status, ws) = post(
        &server,
        &token,
        "/repos/wendy/proj/workspaces",
        json!({ "name": "feature" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{ws}");
    let branch = format!("refs/heads/{}", ws["branch"].as_str().unwrap());
    let has_branch = || async {
        git(&bare, &["rev-parse", "--verify", "-q", &branch])
            .await
            .status
            .success()
    };
    assert!(has_branch().await);

    sqlx::query("UPDATE workspaces SET expires_at = '2000-01-01T00:00:00+00:00'")
        .execute(&server.state.db)
        .await
        .unwrap();
    delta_api::routes::workspaces::expire_workspaces(
        &server.state.db,
        &server.state.repo_host,
        &server.state.workspace_locks,
    )
    .await;

    assert!(!has_branch().await, "expired workspace branch was kept");
    let ws = delta_core::db::workspace::get_by_id(&server.state.db, ws["id"].as_str().unwrap())
        .await
        .unwrap();
    assert_eq!(
        ws.status,
        delta_core::models::workspace::WorkspaceStatus::Expired
    );
}
