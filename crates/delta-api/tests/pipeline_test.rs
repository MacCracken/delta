//! Integration tests for running CI pipelines on demand.

mod common;

use std::time::Duration;

use common::{Server, create_user_and_repo, git_ok, post, start_server};
use reqwest::StatusCode;
use serde_json::{Value, json};

/// Wait until the pipeline leaves the queued/running states; returns it.
async fn finished_pipeline(server: &Server, token: &str, path: &str, id: &str) -> Value {
    let url = format!("{}/api/v1/repos/{path}/pipelines/{id}", server.base);
    for _ in 0..200 {
        let run: Value = reqwest::Client::new()
            .get(&url)
            .bearer_auth(token)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        if !matches!(run["status"].as_str(), Some("queued" | "running")) {
            return run;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("pipeline {id} did not finish");
}

#[tokio::test]
async fn manual_and_workspace_triggers_run_the_workflow() {
    let server = start_server().await;
    let token = create_user_and_repo(&server, "mia", "app", "private").await;
    let bare = server.state.repo_host.repo_path("mia", "app").unwrap();
    let work = tempfile::tempdir().unwrap();
    let dir = work.path();
    git_ok(dir, &["init", "-q", "-b", "main"]).await;
    std::fs::create_dir_all(dir.join(".delta/workflows")).unwrap();
    // Pushes to main don't trigger it: only explicit runs do.
    std::fs::write(
        dir.join(".delta/workflows/release.toml"),
        "name = \"Release\"\n\n[[on]]\npush = { branches = [\"release\"] }\n\n\
         [jobs.build]\nneeds = []\n\n[[jobs.build.steps]]\nname = \"check\"\n\
         run = \"test \\\"$DELTA_TRIGGER\\\" = manual || test \\\"$DELTA_TRIGGER\\\" = workspace\"\n",
    )
    .unwrap();
    git_ok(dir, &["add", "."]).await;
    git_ok(dir, &["commit", "-q", "-m", "init"]).await;
    git_ok(dir, &["push", "-q", bare.to_str().unwrap(), "main"]).await;

    let (status, run) = post(
        &server,
        &token,
        "/repos/mia/app/pipelines",
        json!({ "workflow_name": "Release", "commit_sha": "main" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{run}");
    let id = run["id"].as_str().unwrap();
    let run = finished_pipeline(&server, &token, "mia/app", id).await;
    assert_eq!(run["status"], "passed", "{run}");
    assert_eq!(run["commit_sha"].as_str().unwrap().len(), 40);

    // A workflow that doesn't exist fails instead of staying queued.
    let (status, run) = post(
        &server,
        &token,
        "/repos/mia/app/pipelines",
        json!({ "workflow_name": "Nope", "commit_sha": "main" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let run = finished_pipeline(&server, &token, "mia/app", run["id"].as_str().unwrap()).await;
    assert_eq!(run["status"], "failed", "{run}");

    // Commits must exist in the repository.
    let (status, _) = post(
        &server,
        &token,
        "/repos/mia/app/pipelines",
        json!({ "workflow_name": "Release", "commit_sha": "0".repeat(40) }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // Workspaces run their head commit.
    let (status, ws) = post(
        &server,
        &token,
        "/repos/mia/app/workspaces",
        json!({ "name": "try" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{ws}");
    let (status, run) = post(
        &server,
        &token,
        &format!(
            "/repos/mia/app/workspaces/{}/pipelines",
            ws["id"].as_str().unwrap()
        ),
        json!({ "workflow_name": "release" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{run}");
    let run = finished_pipeline(&server, &token, "mia/app", run["id"].as_str().unwrap()).await;
    assert_eq!(run["status"], "passed", "{run}");
    assert_eq!(run["trigger_type"], "workspace");
}
