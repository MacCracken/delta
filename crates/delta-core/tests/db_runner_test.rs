//! Integration tests for runner DB operations.

mod common;

use delta_core::db;

// --- Runner CRUD ---

#[tokio::test]
async fn test_runner_register_and_get() {
    let pool = common::setup_pool().await;
    let labels = vec!["linux".to_string(), "gpu".to_string()];
    let runner = db::runner::register(&pool, "my-runner", "hash123", &labels)
        .await
        .unwrap();

    assert_eq!(runner.name, "my-runner");
    assert_eq!(runner.labels, vec!["linux", "gpu"]);
    assert_eq!(runner.status, db::runner::RunnerStatus::Online);

    let fetched = db::runner::get(&pool, &runner.id).await.unwrap();
    assert_eq!(fetched.id, runner.id);
    assert_eq!(fetched.name, "my-runner");
}

#[tokio::test]
async fn test_runner_register_conflict_different_token() {
    let pool = common::setup_pool().await;
    let labels = vec!["linux".to_string()];

    db::runner::register(&pool, "my-runner", "hash-a", &labels)
        .await
        .unwrap();

    // Same name, different token should fail
    let result = db::runner::register(&pool, "my-runner", "hash-b", &labels).await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("different token"), "unexpected error: {}", err);
}

#[tokio::test]
async fn test_runner_register_same_name_same_token_updates() {
    let pool = common::setup_pool().await;
    let labels1 = vec!["linux".to_string()];
    let labels2 = vec!["linux".to_string(), "arm64".to_string()];

    let r1 = db::runner::register(&pool, "my-runner", "hash-a", &labels1)
        .await
        .unwrap();

    // Same name, same token should update labels
    let r2 = db::runner::register(&pool, "my-runner", "hash-a", &labels2)
        .await
        .unwrap();

    assert_eq!(r2.name, "my-runner");
    assert_eq!(r2.labels, vec!["linux", "arm64"]);
    // ID stays the same since it's an update
    assert_eq!(r1.id, r2.id);
}

#[tokio::test]
async fn test_runner_list_with_pagination() {
    let pool = common::setup_pool().await;
    let labels = vec![];

    for i in 0..5 {
        db::runner::register(
            &pool,
            &format!("runner-{}", i),
            &format!("hash-{}", i),
            &labels,
        )
        .await
        .unwrap();
    }

    // Page 1: first 2
    let page1 = db::runner::list(&pool, 2, 0).await.unwrap();
    assert_eq!(page1.len(), 2);

    // Page 2: next 2
    let page2 = db::runner::list(&pool, 2, 2).await.unwrap();
    assert_eq!(page2.len(), 2);

    // Page 3: last 1
    let page3 = db::runner::list(&pool, 2, 4).await.unwrap();
    assert_eq!(page3.len(), 1);

    // Beyond: empty
    let page4 = db::runner::list(&pool, 2, 10).await.unwrap();
    assert!(page4.is_empty());
}

#[tokio::test]
async fn test_runner_delete() {
    let pool = common::setup_pool().await;
    let runner = db::runner::register(&pool, "doomed", "hash", &[])
        .await
        .unwrap();

    db::runner::delete(&pool, &runner.id).await.unwrap();

    let result = db::runner::get(&pool, &runner.id).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_runner_delete_not_found() {
    let pool = common::setup_pool().await;
    let result = db::runner::delete(&pool, "nonexistent").await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("not found"));
}

#[tokio::test]
async fn test_runner_heartbeat() {
    let pool = common::setup_pool().await;
    let runner = db::runner::register(&pool, "hb-runner", "hash", &[])
        .await
        .unwrap();
    let initial_hb = runner.last_heartbeat_at.clone();

    // Small delay to ensure timestamp changes
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;

    db::runner::heartbeat(&pool, &runner.id).await.unwrap();

    let updated = db::runner::get(&pool, &runner.id).await.unwrap();
    assert_eq!(updated.status, db::runner::RunnerStatus::Online);
    // Heartbeat should have been updated
    assert_ne!(updated.last_heartbeat_at, initial_hb);
}

#[tokio::test]
async fn test_runner_authenticate_success() {
    let pool = common::setup_pool().await;
    db::runner::register(&pool, "auth-runner", "secret-hash", &[])
        .await
        .unwrap();

    let runner = db::runner::authenticate(&pool, "auth-runner", "secret-hash")
        .await
        .unwrap();
    assert_eq!(runner.name, "auth-runner");
}

#[tokio::test]
async fn test_runner_authenticate_wrong_token() {
    let pool = common::setup_pool().await;
    db::runner::register(&pool, "auth-runner", "correct-hash", &[])
        .await
        .unwrap();

    let result = db::runner::authenticate(&pool, "auth-runner", "wrong-hash").await;
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("invalid runner credentials")
    );
}

// --- Job queue ---

/// Helper: set up a pipeline + job_run so we can enqueue jobs (foreign keys).
async fn create_pipeline_and_job(pool: &sqlx::SqlitePool) -> (String, String, String) {
    let user = common::create_test_user(pool).await;
    let repo = common::create_test_repo(pool, &user.id.to_string()).await;
    let repo_id = repo.id.to_string();

    let pipeline =
        db::pipeline::create_pipeline(pool, &repo_id, "ci", "push", Some("main"), "abc123")
            .await
            .unwrap();

    let job = db::pipeline::create_job(pool, &pipeline.id, "build")
        .await
        .unwrap();

    (repo_id, pipeline.id.clone(), job.id.clone())
}

#[tokio::test]
async fn test_enqueue_and_get_job() {
    let pool = common::setup_pool().await;
    let (repo_id, pipeline_id, job_run_id) = create_pipeline_and_job(&pool).await;
    let labels = vec!["linux".to_string()];

    let queued = db::runner::enqueue_job(
        &pool,
        "q1",
        &job_run_id,
        &pipeline_id,
        &repo_id,
        &labels,
        r#"{"hello":"world"}"#,
    )
    .await
    .unwrap();

    assert_eq!(queued.id, "q1");
    assert_eq!(queued.status, "pending");
    assert_eq!(queued.labels, vec!["linux"]);
    assert!(queued.claimed_by.is_none());

    let fetched = db::runner::get_queued_job(&pool, "q1").await.unwrap();
    assert_eq!(fetched.id, queued.id);
}

#[tokio::test]
async fn test_get_queued_job_by_job_run_id() {
    let pool = common::setup_pool().await;
    let (repo_id, pipeline_id, job_run_id) = create_pipeline_and_job(&pool).await;

    db::runner::enqueue_job(&pool, "q2", &job_run_id, &pipeline_id, &repo_id, &[], "{}")
        .await
        .unwrap();

    let fetched = db::runner::get_queued_job_by_job_run_id(&pool, &job_run_id)
        .await
        .unwrap();
    assert_eq!(fetched.id, "q2");
    assert_eq!(fetched.job_run_id, job_run_id);
}

#[tokio::test]
async fn test_poll_job_matches_labels() {
    let pool = common::setup_pool().await;
    let (repo_id, pipeline_id, job_run_id) = create_pipeline_and_job(&pool).await;

    let runner = db::runner::register(&pool, "poll-runner", "hash", &["linux".to_string()])
        .await
        .unwrap();

    // Enqueue a job requiring "linux" label
    db::runner::enqueue_job(
        &pool,
        "q3",
        &job_run_id,
        &pipeline_id,
        &repo_id,
        &["linux".to_string()],
        "{}",
    )
    .await
    .unwrap();

    // Runner with "linux" label should match
    let polled = db::runner::poll_job(&pool, &runner.id, &["linux".to_string()])
        .await
        .unwrap();
    assert!(polled.is_some());
    let polled = polled.unwrap();
    assert_eq!(polled.id, "q3");
    assert_eq!(polled.status, "claimed");
    assert_eq!(polled.claimed_by.as_deref(), Some(&*runner.id));
}

#[tokio::test]
async fn test_poll_job_no_match() {
    let pool = common::setup_pool().await;
    let (repo_id, pipeline_id, job_run_id) = create_pipeline_and_job(&pool).await;

    let runner = db::runner::register(&pool, "arm-runner", "hash", &["arm64".to_string()])
        .await
        .unwrap();

    // Enqueue a job requiring "linux" — runner only has "arm64"
    db::runner::enqueue_job(
        &pool,
        "q4",
        &job_run_id,
        &pipeline_id,
        &repo_id,
        &["linux".to_string()],
        "{}",
    )
    .await
    .unwrap();

    let polled = db::runner::poll_job(&pool, &runner.id, &["arm64".to_string()])
        .await
        .unwrap();
    assert!(polled.is_none());
}

#[tokio::test]
async fn test_poll_job_empty_labels_matches_any() {
    let pool = common::setup_pool().await;
    let (repo_id, pipeline_id, job_run_id) = create_pipeline_and_job(&pool).await;

    let runner = db::runner::register(&pool, "any-runner", "hash", &[])
        .await
        .unwrap();

    // Job with empty labels should match any runner
    db::runner::enqueue_job(&pool, "q5", &job_run_id, &pipeline_id, &repo_id, &[], "{}")
        .await
        .unwrap();

    let polled = db::runner::poll_job(&pool, &runner.id, &[]).await.unwrap();
    assert!(polled.is_some());
}

#[tokio::test]
async fn test_complete_queued_job() {
    let pool = common::setup_pool().await;
    let (repo_id, pipeline_id, job_run_id) = create_pipeline_and_job(&pool).await;

    let runner = db::runner::register(&pool, "comp-runner", "hash", &[])
        .await
        .unwrap();

    db::runner::enqueue_job(&pool, "q6", &job_run_id, &pipeline_id, &repo_id, &[], "{}")
        .await
        .unwrap();

    // Claim the job
    db::runner::poll_job(&pool, &runner.id, &[]).await.unwrap();

    // Complete it
    db::runner::complete_queued_job(&pool, "q6", &runner.id)
        .await
        .unwrap();

    let job = db::runner::get_queued_job(&pool, "q6").await.unwrap();
    assert_eq!(job.status, "completed");
}

#[tokio::test]
async fn test_complete_queued_job_wrong_runner() {
    let pool = common::setup_pool().await;
    let (repo_id, pipeline_id, job_run_id) = create_pipeline_and_job(&pool).await;

    let runner = db::runner::register(&pool, "owner-runner", "hash1", &[])
        .await
        .unwrap();
    let other = db::runner::register(&pool, "other-runner", "hash2", &[])
        .await
        .unwrap();

    db::runner::enqueue_job(&pool, "q7", &job_run_id, &pipeline_id, &repo_id, &[], "{}")
        .await
        .unwrap();

    // Owner claims
    db::runner::poll_job(&pool, &runner.id, &[]).await.unwrap();

    // Other runner tries to complete — should fail
    let result = db::runner::complete_queued_job(&pool, "q7", &other.id).await;
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("not claimed by this runner")
    );
}

#[tokio::test]
async fn test_reclaim_stale_jobs() {
    let pool = common::setup_pool().await;
    let (repo_id, pipeline_id, job_run_id) = create_pipeline_and_job(&pool).await;

    let runner = db::runner::register(&pool, "stale-runner", "hash", &[])
        .await
        .unwrap();

    db::runner::enqueue_job(&pool, "q8", &job_run_id, &pipeline_id, &repo_id, &[], "{}")
        .await
        .unwrap();

    // Claim the job
    db::runner::poll_job(&pool, &runner.id, &[]).await.unwrap();

    // Set heartbeat to very old
    sqlx::query(
        "UPDATE runners SET last_heartbeat_at = datetime('now', '-120 minutes') WHERE id = ?",
    )
    .bind(&runner.id)
    .execute(&pool)
    .await
    .unwrap();

    // Reclaim with 60 min threshold
    let reclaimed = db::runner::reclaim_stale_jobs(&pool, 60).await.unwrap();
    assert_eq!(reclaimed, 1);

    // Job should be pending again
    let job = db::runner::get_queued_job(&pool, "q8").await.unwrap();
    assert_eq!(job.status, "pending");
    assert!(job.claimed_by.is_none());
}

#[tokio::test]
async fn test_set_job_runner() {
    let pool = common::setup_pool().await;
    let (_repo_id, _pipeline_id, job_run_id) = create_pipeline_and_job(&pool).await;

    db::runner::set_job_runner(&pool, &job_run_id, "my-runner")
        .await
        .unwrap();

    let job = db::pipeline::get_job(&pool, &job_run_id).await.unwrap();
    assert_eq!(job.runner.as_deref(), Some("my-runner"));
}
