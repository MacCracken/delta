//! Tests for pipeline, job, step log, and secret DB operations.

mod common;

use delta_core::db;

// --- Pipeline tests ---

#[tokio::test]
async fn test_pipeline_create_and_get() {
    let pool = common::setup_pool().await;
    let user = common::create_test_user(&pool).await;
    let repo = common::create_test_repo(&pool, &user.id.to_string()).await;
    let repo_id = repo.id.to_string();

    let pipeline =
        db::pipeline::create_pipeline(&pool, &repo_id, "ci", "push", Some("main"), "abc123")
            .await
            .unwrap();

    assert_eq!(pipeline.workflow_name, "ci");
    assert_eq!(pipeline.trigger_type, "push");
    assert_eq!(pipeline.commit_sha, "abc123");
    assert_eq!(pipeline.status, db::pipeline::RunStatus::Queued);

    let fetched = db::pipeline::get_pipeline(&pool, &pipeline.id)
        .await
        .unwrap();
    assert_eq!(fetched.id, pipeline.id);
}

#[tokio::test]
async fn test_pipeline_update_status() {
    let pool = common::setup_pool().await;
    let user = common::create_test_user(&pool).await;
    let repo = common::create_test_repo(&pool, &user.id.to_string()).await;
    let repo_id = repo.id.to_string();

    let pipeline = db::pipeline::create_pipeline(&pool, &repo_id, "ci", "push", None, "sha1")
        .await
        .unwrap();

    let updated =
        db::pipeline::update_pipeline_status(&pool, &pipeline.id, db::pipeline::RunStatus::Running)
            .await
            .unwrap();
    assert_eq!(updated.status, db::pipeline::RunStatus::Running);
}

#[tokio::test]
async fn test_pipeline_list() {
    let pool = common::setup_pool().await;
    let user = common::create_test_user(&pool).await;
    let repo = common::create_test_repo(&pool, &user.id.to_string()).await;
    let repo_id = repo.id.to_string();

    db::pipeline::create_pipeline(&pool, &repo_id, "ci", "push", None, "sha1")
        .await
        .unwrap();
    db::pipeline::create_pipeline(&pool, &repo_id, "deploy", "manual", None, "sha2")
        .await
        .unwrap();

    let all = db::pipeline::list_pipelines(&pool, &repo_id, None, 50)
        .await
        .unwrap();
    assert_eq!(all.len(), 2);
}

#[tokio::test]
async fn test_pipeline_jobs_and_logs() {
    let pool = common::setup_pool().await;
    let user = common::create_test_user(&pool).await;
    let repo = common::create_test_repo(&pool, &user.id.to_string()).await;
    let repo_id = repo.id.to_string();

    let pipeline = db::pipeline::create_pipeline(&pool, &repo_id, "ci", "push", None, "sha1")
        .await
        .unwrap();

    let job = db::pipeline::create_job(&pool, &pipeline.id, "build")
        .await
        .unwrap();
    assert_eq!(job.job_name, "build");

    db::pipeline::append_step_log(&pool, &job.id, "compile", 0, "output here", "passed")
        .await
        .unwrap();

    let logs = db::pipeline::get_step_logs(&pool, &job.id).await.unwrap();
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].step_name, "compile");

    let jobs = db::pipeline::list_jobs(&pool, &pipeline.id).await.unwrap();
    assert_eq!(jobs.len(), 1);
}

// --- Secret tests ---

#[tokio::test]
async fn test_secret_set_and_list() {
    let pool = common::setup_pool().await;
    let user = common::create_test_user(&pool).await;
    let repo = common::create_test_repo(&pool, &user.id.to_string()).await;
    let repo_id = repo.id.to_string();

    db::secret::set(&pool, &repo_id, "API_KEY", "encrypted-value-1")
        .await
        .unwrap();
    db::secret::set(&pool, &repo_id, "DB_PASSWORD", "encrypted-value-2")
        .await
        .unwrap();

    let secrets = db::secret::list(&pool, &repo_id).await.unwrap();
    assert_eq!(secrets.len(), 2);
    let names: Vec<&str> = secrets.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"API_KEY"));
    assert!(names.contains(&"DB_PASSWORD"));
}

#[tokio::test]
async fn test_secret_upsert() {
    let pool = common::setup_pool().await;
    let user = common::create_test_user(&pool).await;
    let repo = common::create_test_repo(&pool, &user.id.to_string()).await;
    let repo_id = repo.id.to_string();

    db::secret::set(&pool, &repo_id, "KEY", "value1")
        .await
        .unwrap();
    db::secret::set(&pool, &repo_id, "KEY", "value2")
        .await
        .unwrap();

    let secrets = db::secret::list(&pool, &repo_id).await.unwrap();
    assert_eq!(secrets.len(), 1);
}

#[tokio::test]
async fn test_secret_get_all_values() {
    let pool = common::setup_pool().await;
    let user = common::create_test_user(&pool).await;
    let repo = common::create_test_repo(&pool, &user.id.to_string()).await;
    let repo_id = repo.id.to_string();

    db::secret::set(&pool, &repo_id, "A", "enc_a")
        .await
        .unwrap();
    db::secret::set(&pool, &repo_id, "B", "enc_b")
        .await
        .unwrap();

    let values = db::secret::get_all_values(&pool, &repo_id).await.unwrap();
    assert_eq!(values.len(), 2);
    assert!(
        values
            .iter()
            .any(|(name, val)| name == "A" && val == "enc_a")
    );
    assert!(
        values
            .iter()
            .any(|(name, val)| name == "B" && val == "enc_b")
    );
}

#[tokio::test]
async fn test_secret_delete() {
    let pool = common::setup_pool().await;
    let user = common::create_test_user(&pool).await;
    let repo = common::create_test_repo(&pool, &user.id.to_string()).await;
    let repo_id = repo.id.to_string();

    db::secret::set(&pool, &repo_id, "TO_DELETE", "val")
        .await
        .unwrap();
    db::secret::delete(&pool, &repo_id, "TO_DELETE")
        .await
        .unwrap();

    let secrets = db::secret::list(&pool, &repo_id).await.unwrap();
    assert!(secrets.is_empty());
}

#[tokio::test]
async fn test_secret_delete_not_found() {
    let pool = common::setup_pool().await;
    let user = common::create_test_user(&pool).await;
    let repo = common::create_test_repo(&pool, &user.id.to_string()).await;
    let repo_id = repo.id.to_string();

    let result = db::secret::delete(&pool, &repo_id, "nonexistent").await;
    assert!(result.is_err());
}

// --- DB init_pool test ---

#[tokio::test]
async fn test_db_init_pool() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let path = tmp.path().to_str().unwrap();
    let url = format!("sqlite://{}", path);

    let pool = db::init_pool(&url).await.unwrap();
    let user = db::user::create(&pool, "inituser", "init@test.com", "pass", false)
        .await
        .unwrap();
    assert_eq!(user.username, "inituser");
}
