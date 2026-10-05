//! End-to-end tests of pipeline execution against a real database.

use std::collections::HashMap;

use delta_ci::executor::SandboxMode;
use delta_ci::runner::{PipelineContext, run_push_pipelines};
use delta_core::db;

const WORKFLOW: &str = r#"
name = "CI"

[[on]]
push = { branches = ["main"] }

[jobs.build]
needs = []

[[jobs.build.steps]]
name = "Fail"
run = "exit 3"

[jobs.deploy]
needs = ["build"]

[[jobs.deploy.steps]]
name = "Deploy"
run = "touch deployed"

[jobs.lint]
needs = []

[[jobs.lint.steps]]
name = "Lint"
run = "touch linted"
"#;

#[tokio::test]
async fn failed_jobs_block_their_dependents_only() {
    let tmp = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}", tmp.path().join("delta.db").display());
    let pool = db::init_pool(&url).await.unwrap();
    let user = db::user::create(&pool, "u", "u@example.com", "pw", false)
        .await
        .unwrap();
    let repo = db::repo::create(
        &pool,
        &user.id.to_string(),
        "r",
        None,
        delta_core::models::repo::Visibility::Public,
    )
    .await
    .unwrap();
    let repo_id = repo.id.to_string();

    let work = tmp.path().join("checkout");
    std::fs::create_dir_all(work.join(".delta/workflows")).unwrap();
    std::fs::write(work.join(".delta/workflows/ci.toml"), WORKFLOW).unwrap();

    let secrets = HashMap::new();
    let ctx = PipelineContext {
        pool: &pool,
        repo_id: &repo_id,
        repo_path: &work,
        home_dir: None,
        commit_sha: "0123456789abcdef0123456789abcdef01234567",
        secrets: &secrets,
        streams: None,
        sandbox: SandboxMode::None,
        runners_enabled: false,
    };
    run_push_pipelines(&ctx, "main").await;

    // The dependent of the failed job never ran; the independent job did.
    assert!(!work.join("deployed").exists());
    assert!(work.join("linted").exists());

    let pipelines = db::pipeline::list_pipelines(&pool, &repo_id, None, 10)
        .await
        .unwrap();
    assert_eq!(pipelines.len(), 1);
    assert_eq!(pipelines[0].status, db::pipeline::RunStatus::Failed);
    let jobs = db::pipeline::list_jobs(&pool, &pipelines[0].id)
        .await
        .unwrap();
    let status = |name: &str| {
        jobs.iter()
            .find(|j| j.job_name == name)
            .map(|j| j.status)
    };
    assert_eq!(status("build"), Some(db::pipeline::RunStatus::Failed));
    assert_eq!(status("deploy"), Some(db::pipeline::RunStatus::Failed));
    assert_eq!(status("lint"), Some(db::pipeline::RunStatus::Passed));
}
