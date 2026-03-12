use delta_core::db;
use delta_core::models::repo::Visibility;

async fn setup_pool() -> sqlx::SqlitePool {
    let pool = sqlx::SqlitePool::connect("sqlite::memory:")
        .await
        .expect("failed to connect to in-memory db");

    for migration in [
        include_str!("../migrations/001_initial.sql"),
        include_str!("../migrations/002_git_protocol.sql"),
        include_str!("../migrations/003_pull_requests.sql"),
        include_str!("../migrations/004_cicd.sql"),
        include_str!("../migrations/005_registry.sql"),
        include_str!("../migrations/006_collaborators.sql"),
        include_str!("../migrations/007_forks_and_templates.sql"),
        include_str!("../migrations/008_lfs.sql"),
        include_str!("../migrations/009_cascade_fixes.sql"),
    ] {
        sqlx::query(migration)
            .execute(&pool)
            .await
            .expect("failed to run migration");
    }

    pool
}

async fn create_test_user(pool: &sqlx::SqlitePool) -> delta_core::models::user::User {
    db::user::create(pool, "testuser", "test@example.com", "hashedpw", false)
        .await
        .expect("failed to create user")
}

async fn create_test_repo(
    pool: &sqlx::SqlitePool,
    owner_id: &str,
) -> delta_core::models::repo::Repository {
    db::repo::create(pool, owner_id, "testrepo", Some("desc"), Visibility::Public)
        .await
        .expect("failed to create repo")
}

// --- Artifact tests ---

#[tokio::test]
async fn test_artifact_create_and_get() {
    let pool = setup_pool().await;
    let user = create_test_user(&pool).await;
    let repo = create_test_repo(&pool, &user.id.to_string()).await;

    let artifact = db::artifact::create(
        &pool,
        &db::artifact::CreateArtifactParams {
            repo_id: &repo.id.to_string(),
            pipeline_id: None,
            name: "build.tar.gz",
            version: Some("1.0.0"),
            artifact_type: "generic",
            content_hash: "abc123",
            size_bytes: 1024,
            metadata: Some("{\"arch\": \"amd64\"}"),
        },
    )
    .await
    .expect("failed to create artifact");

    assert_eq!(artifact.name, "build.tar.gz");
    assert_eq!(artifact.version.as_deref(), Some("1.0.0"));
    assert_eq!(artifact.size_bytes, 1024);
    assert_eq!(artifact.download_count, 0);

    let fetched = db::artifact::get(&pool, &artifact.id).await.unwrap();
    assert_eq!(fetched.id, artifact.id);
    assert_eq!(fetched.content_hash, "abc123");
}

#[tokio::test]
async fn test_artifact_list_for_repo() {
    let pool = setup_pool().await;
    let user = create_test_user(&pool).await;
    let repo = create_test_repo(&pool, &user.id.to_string()).await;

    for i in 0..3 {
        db::artifact::create(
            &pool,
            &db::artifact::CreateArtifactParams {
                repo_id: &repo.id.to_string(),
                pipeline_id: None,
                name: &format!("artifact-{}", i),
                version: None,
                artifact_type: "generic",
                content_hash: &format!("hash{}", i),
                size_bytes: 100,
                metadata: None,
            },
        )
        .await
        .unwrap();
    }

    let artifacts = db::artifact::list_for_repo(&pool, &repo.id.to_string())
        .await
        .unwrap();
    assert_eq!(artifacts.len(), 3);
}

#[tokio::test]
async fn test_artifact_increment_download() {
    let pool = setup_pool().await;
    let user = create_test_user(&pool).await;
    let repo = create_test_repo(&pool, &user.id.to_string()).await;

    let artifact = db::artifact::create(
        &pool,
        &db::artifact::CreateArtifactParams {
            repo_id: &repo.id.to_string(),
            pipeline_id: None,
            name: "file.zip",
            version: None,
            artifact_type: "generic",
            content_hash: "hash1",
            size_bytes: 50,
            metadata: None,
        },
    )
    .await
    .unwrap();

    db::artifact::increment_download(&pool, &artifact.id)
        .await
        .unwrap();
    db::artifact::increment_download(&pool, &artifact.id)
        .await
        .unwrap();

    let fetched = db::artifact::get(&pool, &artifact.id).await.unwrap();
    assert_eq!(fetched.download_count, 2);
}

#[tokio::test]
async fn test_artifact_delete() {
    let pool = setup_pool().await;
    let user = create_test_user(&pool).await;
    let repo = create_test_repo(&pool, &user.id.to_string()).await;

    let artifact = db::artifact::create(
        &pool,
        &db::artifact::CreateArtifactParams {
            repo_id: &repo.id.to_string(),
            pipeline_id: None,
            name: "to-delete",
            version: None,
            artifact_type: "generic",
            content_hash: "delhash",
            size_bytes: 10,
            metadata: None,
        },
    )
    .await
    .unwrap();

    db::artifact::delete(&pool, &artifact.id).await.unwrap();

    let result = db::artifact::get(&pool, &artifact.id).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_artifact_delete_not_found() {
    let pool = setup_pool().await;
    let result = db::artifact::delete(&pool, "nonexistent").await;
    assert!(result.is_err());
}

// --- Release tests ---

#[tokio::test]
async fn test_release_create_and_get() {
    let pool = setup_pool().await;
    let user = create_test_user(&pool).await;
    let repo = create_test_repo(&pool, &user.id.to_string()).await;

    let release = db::release::create(
        &pool,
        &db::release::CreateReleaseParams {
            repo_id: &repo.id.to_string(),
            tag_name: "2026.1.1",
            name: "Release 2026.1.1",
            body: Some("First release!"),
            is_draft: false,
            is_prerelease: false,
            author_id: &user.id.to_string(),
        },
    )
    .await
    .expect("failed to create release");

    assert_eq!(release.tag_name, "2026.1.1");
    assert_eq!(release.name, "Release 2026.1.1");
    assert_eq!(release.body.as_deref(), Some("First release!"));
    assert!(!release.is_draft);
    assert!(!release.is_prerelease);
}

#[tokio::test]
async fn test_release_get_by_tag() {
    let pool = setup_pool().await;
    let user = create_test_user(&pool).await;
    let repo = create_test_repo(&pool, &user.id.to_string()).await;
    let repo_id = repo.id.to_string();

    db::release::create(
        &pool,
        &db::release::CreateReleaseParams {
            repo_id: &repo_id,
            tag_name: "2026.2.1",
            name: "2026.2.1",
            body: None,
            is_draft: false,
            is_prerelease: true,
            author_id: &user.id.to_string(),
        },
    )
    .await
    .unwrap();

    let release = db::release::get_by_tag(&pool, &repo_id, "2026.2.1")
        .await
        .unwrap();
    assert_eq!(release.tag_name, "2026.2.1");
    assert!(release.is_prerelease);
}

#[tokio::test]
async fn test_release_get_by_tag_not_found() {
    let pool = setup_pool().await;
    let result = db::release::get_by_tag(&pool, "norepo", "notag").await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_release_list_for_repo() {
    let pool = setup_pool().await;
    let user = create_test_user(&pool).await;
    let repo = create_test_repo(&pool, &user.id.to_string()).await;
    let repo_id = repo.id.to_string();

    for i in 0..3 {
        db::release::create(
            &pool,
            &db::release::CreateReleaseParams {
                repo_id: &repo_id,
                tag_name: &format!("2026.{}.1", i + 1),
                name: &format!("Release {}", i),
                body: None,
                is_draft: false,
                is_prerelease: false,
                author_id: &user.id.to_string(),
            },
        )
        .await
        .unwrap();
    }

    let releases = db::release::list_for_repo(&pool, &repo_id).await.unwrap();
    assert_eq!(releases.len(), 3);
}

#[tokio::test]
async fn test_release_duplicate_tag_fails() {
    let pool = setup_pool().await;
    let user = create_test_user(&pool).await;
    let repo = create_test_repo(&pool, &user.id.to_string()).await;
    let repo_id = repo.id.to_string();

    db::release::create(
        &pool,
        &db::release::CreateReleaseParams {
            repo_id: &repo_id,
            tag_name: "2026.3.1",
            name: "First",
            body: None,
            is_draft: false,
            is_prerelease: false,
            author_id: &user.id.to_string(),
        },
    )
    .await
    .unwrap();

    let result = db::release::create(
        &pool,
        &db::release::CreateReleaseParams {
            repo_id: &repo_id,
            tag_name: "2026.3.1",
            name: "Duplicate",
            body: None,
            is_draft: false,
            is_prerelease: false,
            author_id: &user.id.to_string(),
        },
    )
    .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_release_delete() {
    let pool = setup_pool().await;
    let user = create_test_user(&pool).await;
    let repo = create_test_repo(&pool, &user.id.to_string()).await;
    let repo_id = repo.id.to_string();

    let release = db::release::create(
        &pool,
        &db::release::CreateReleaseParams {
            repo_id: &repo_id,
            tag_name: "2026.4.1",
            name: "To delete",
            body: None,
            is_draft: true,
            is_prerelease: false,
            author_id: &user.id.to_string(),
        },
    )
    .await
    .unwrap();

    db::release::delete(&pool, &release.id).await.unwrap();
    let result = db::release::get(&pool, &release.id).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_release_delete_not_found() {
    let pool = setup_pool().await;
    let result = db::release::delete(&pool, "nonexistent").await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_release_attach_asset() {
    let pool = setup_pool().await;
    let user = create_test_user(&pool).await;
    let repo = create_test_repo(&pool, &user.id.to_string()).await;
    let repo_id = repo.id.to_string();

    let release = db::release::create(
        &pool,
        &db::release::CreateReleaseParams {
            repo_id: &repo_id,
            tag_name: "2026.5.1",
            name: "With asset",
            body: None,
            is_draft: false,
            is_prerelease: false,
            author_id: &user.id.to_string(),
        },
    )
    .await
    .unwrap();

    let artifact = db::artifact::create(
        &pool,
        &db::artifact::CreateArtifactParams {
            repo_id: &repo_id,
            pipeline_id: None,
            name: "asset-file",
            version: None,
            artifact_type: "generic",
            content_hash: "assethash",
            size_bytes: 500,
            metadata: None,
        },
    )
    .await
    .unwrap();

    db::release::attach_asset(&pool, &release.id, &artifact.id, Some("binary"))
        .await
        .unwrap();
}

// --- Secret tests ---

#[tokio::test]
async fn test_secret_set_and_list() {
    let pool = setup_pool().await;
    let user = create_test_user(&pool).await;
    let repo = create_test_repo(&pool, &user.id.to_string()).await;
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
    let pool = setup_pool().await;
    let user = create_test_user(&pool).await;
    let repo = create_test_repo(&pool, &user.id.to_string()).await;
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
    let pool = setup_pool().await;
    let user = create_test_user(&pool).await;
    let repo = create_test_repo(&pool, &user.id.to_string()).await;
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
    let pool = setup_pool().await;
    let user = create_test_user(&pool).await;
    let repo = create_test_repo(&pool, &user.id.to_string()).await;
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
    let pool = setup_pool().await;
    let user = create_test_user(&pool).await;
    let repo = create_test_repo(&pool, &user.id.to_string()).await;
    let repo_id = repo.id.to_string();

    let result = db::secret::delete(&pool, &repo_id, "nonexistent").await;
    assert!(result.is_err());
}

// --- Audit log tests ---

#[tokio::test]
async fn test_audit_log_and_list() {
    let pool = setup_pool().await;
    let user = create_test_user(&pool).await;
    let user_id = user.id.to_string();

    db::audit::log(
        &pool,
        Some(&user_id),
        "create",
        "repository",
        Some("repo-123"),
        Some("created repo"),
        Some("127.0.0.1"),
    )
    .await
    .unwrap();

    db::audit::log(
        &pool,
        Some(&user_id),
        "delete",
        "repository",
        Some("repo-456"),
        None,
        None,
    )
    .await
    .unwrap();

    let entries = db::audit::list(&pool, Some(&user_id), None, 50, 0)
        .await
        .unwrap();
    assert_eq!(entries.len(), 2);
}

#[tokio::test]
async fn test_audit_log_filter_by_resource_type() {
    let pool = setup_pool().await;
    let user = create_test_user(&pool).await;
    let user_id = user.id.to_string();

    db::audit::log(
        &pool,
        Some(&user_id),
        "create",
        "repository",
        None,
        None,
        None,
    )
    .await
    .unwrap();
    db::audit::log(&pool, Some(&user_id), "login", "session", None, None, None)
        .await
        .unwrap();

    let repo_entries = db::audit::list(&pool, Some(&user_id), Some("repository"), 50, 0)
        .await
        .unwrap();
    assert_eq!(repo_entries.len(), 1);
    assert_eq!(repo_entries[0].action, "create");
}

#[tokio::test]
async fn test_audit_log_filter_resource_type_only() {
    let pool = setup_pool().await;

    db::audit::log(&pool, None, "system", "config", None, None, None)
        .await
        .unwrap();

    let entries = db::audit::list(&pool, None, Some("config"), 50, 0)
        .await
        .unwrap();
    assert_eq!(entries.len(), 1);
}

#[tokio::test]
async fn test_audit_log_list_all() {
    let pool = setup_pool().await;

    db::audit::log(&pool, None, "boot", "system", None, None, None)
        .await
        .unwrap();

    let entries = db::audit::list(&pool, None, None, 50, 0).await.unwrap();
    assert_eq!(entries.len(), 1);
}

#[tokio::test]
async fn test_audit_log_pagination() {
    let pool = setup_pool().await;

    for i in 0..5 {
        db::audit::log(
            &pool,
            None,
            &format!("action-{}", i),
            "test",
            None,
            None,
            None,
        )
        .await
        .unwrap();
    }

    let page1 = db::audit::list(&pool, None, None, 2, 0).await.unwrap();
    assert_eq!(page1.len(), 2);

    let page2 = db::audit::list(&pool, None, None, 2, 2).await.unwrap();
    assert_eq!(page2.len(), 2);

    let page3 = db::audit::list(&pool, None, None, 2, 4).await.unwrap();
    assert_eq!(page3.len(), 1);
}

// --- Pipeline DB tests ---

#[tokio::test]
async fn test_pipeline_create_and_get() {
    let pool = setup_pool().await;
    let user = create_test_user(&pool).await;
    let repo = create_test_repo(&pool, &user.id.to_string()).await;
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
    let pool = setup_pool().await;
    let user = create_test_user(&pool).await;
    let repo = create_test_repo(&pool, &user.id.to_string()).await;
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
    let pool = setup_pool().await;
    let user = create_test_user(&pool).await;
    let repo = create_test_repo(&pool, &user.id.to_string()).await;
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
    let pool = setup_pool().await;
    let user = create_test_user(&pool).await;
    let repo = create_test_repo(&pool, &user.id.to_string()).await;
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

// --- DB init_pool test ---

#[tokio::test]
async fn test_db_init_pool() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let path = tmp.path().to_str().unwrap();
    let url = format!("sqlite://{}", path);

    let pool = db::init_pool(&url).await.unwrap();
    // Verify we can use it
    let user = db::user::create(&pool, "inituser", "init@test.com", "pass", false)
        .await
        .unwrap();
    assert_eq!(user.username, "inituser");
}
