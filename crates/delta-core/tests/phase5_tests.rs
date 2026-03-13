//! Phase 5 tests: collaborator, audit, signing, ssh_key, status_check, LFS,
//! retention, and ark_package DB modules.

use delta_core::db;
use delta_core::models::collaborator::CollaboratorRole;
use delta_core::models::pull_request::CheckState;
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
        include_str!("../migrations/010_search.sql"),
        include_str!("../migrations/011_federation.sql"),
        include_str!("../migrations/012_encryption.sql"),
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

async fn create_second_user(pool: &sqlx::SqlitePool) -> delta_core::models::user::User {
    db::user::create(pool, "otheruser", "other@example.com", "hashedpw", false)
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

// --- Collaborator tests ---

#[tokio::test]
async fn test_collaborator_set_and_get() {
    let pool = setup_pool().await;
    let user = create_test_user(&pool).await;
    let other = create_second_user(&pool).await;
    let repo = create_test_repo(&pool, &user.id.to_string()).await;

    let collab = db::collaborator::set(
        &pool,
        &repo.id.to_string(),
        &other.id.to_string(),
        CollaboratorRole::Write,
    )
    .await
    .expect("failed to set collaborator");

    assert_eq!(collab.role, CollaboratorRole::Write);
    assert_eq!(collab.user_id, other.id);
}

#[tokio::test]
async fn test_collaborator_get_role() {
    let pool = setup_pool().await;
    let user = create_test_user(&pool).await;
    let other = create_second_user(&pool).await;
    let repo = create_test_repo(&pool, &user.id.to_string()).await;

    // No role initially
    let role = db::collaborator::get_role(&pool, &repo.id.to_string(), &other.id.to_string())
        .await
        .expect("get_role failed");
    assert!(role.is_none());

    // Set and check
    db::collaborator::set(
        &pool,
        &repo.id.to_string(),
        &other.id.to_string(),
        CollaboratorRole::Admin,
    )
    .await
    .unwrap();

    let role = db::collaborator::get_role(&pool, &repo.id.to_string(), &other.id.to_string())
        .await
        .expect("get_role failed");
    assert_eq!(role, Some(CollaboratorRole::Admin));
}

#[tokio::test]
async fn test_collaborator_list_for_repo() {
    let pool = setup_pool().await;
    let user = create_test_user(&pool).await;
    let other = create_second_user(&pool).await;
    let repo = create_test_repo(&pool, &user.id.to_string()).await;

    // Empty initially
    let list = db::collaborator::list_for_repo(&pool, &repo.id.to_string())
        .await
        .unwrap();
    assert!(list.is_empty());

    db::collaborator::set(
        &pool,
        &repo.id.to_string(),
        &other.id.to_string(),
        CollaboratorRole::Read,
    )
    .await
    .unwrap();

    let list = db::collaborator::list_for_repo(&pool, &repo.id.to_string())
        .await
        .unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].role, CollaboratorRole::Read);
}

#[tokio::test]
async fn test_collaborator_update_role() {
    let pool = setup_pool().await;
    let user = create_test_user(&pool).await;
    let other = create_second_user(&pool).await;
    let repo = create_test_repo(&pool, &user.id.to_string()).await;

    db::collaborator::set(
        &pool,
        &repo.id.to_string(),
        &other.id.to_string(),
        CollaboratorRole::Read,
    )
    .await
    .unwrap();

    // Update to Write
    let updated = db::collaborator::set(
        &pool,
        &repo.id.to_string(),
        &other.id.to_string(),
        CollaboratorRole::Write,
    )
    .await
    .unwrap();
    assert_eq!(updated.role, CollaboratorRole::Write);

    // Should still be only 1 collaborator
    let list = db::collaborator::list_for_repo(&pool, &repo.id.to_string())
        .await
        .unwrap();
    assert_eq!(list.len(), 1);
}

#[tokio::test]
async fn test_collaborator_remove() {
    let pool = setup_pool().await;
    let user = create_test_user(&pool).await;
    let other = create_second_user(&pool).await;
    let repo = create_test_repo(&pool, &user.id.to_string()).await;

    db::collaborator::set(
        &pool,
        &repo.id.to_string(),
        &other.id.to_string(),
        CollaboratorRole::Write,
    )
    .await
    .unwrap();

    db::collaborator::remove(&pool, &repo.id.to_string(), &other.id.to_string())
        .await
        .expect("failed to remove");

    let role = db::collaborator::get_role(&pool, &repo.id.to_string(), &other.id.to_string())
        .await
        .unwrap();
    assert!(role.is_none());
}

#[tokio::test]
async fn test_collaborator_remove_nonexistent() {
    let pool = setup_pool().await;
    let user = create_test_user(&pool).await;
    let repo = create_test_repo(&pool, &user.id.to_string()).await;

    let result = db::collaborator::remove(&pool, &repo.id.to_string(), "nonexistent-id").await;
    assert!(result.is_err());
}

// --- Audit tests ---

#[tokio::test]
async fn test_audit_log_and_list() {
    let pool = setup_pool().await;
    let user = create_test_user(&pool).await;
    let uid = user.id.to_string();

    db::audit::log(
        &pool,
        Some(&uid),
        "create_repo",
        "repository",
        Some("repo-123"),
        Some("created testrepo"),
        Some("127.0.0.1"),
    )
    .await
    .expect("failed to log audit");

    let entries = db::audit::list(&pool, Some(&uid), None, 10, 0)
        .await
        .expect("failed to list audit");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].action, "create_repo");
    assert_eq!(entries[0].resource_type, "repository");
    assert_eq!(entries[0].details.as_deref(), Some("created testrepo"));
}

#[tokio::test]
async fn test_audit_list_by_resource_type() {
    let pool = setup_pool().await;
    let user = create_test_user(&pool).await;
    let uid = user.id.to_string();

    db::audit::log(
        &pool,
        Some(&uid),
        "create_repo",
        "repository",
        None,
        None,
        None,
    )
    .await
    .unwrap();
    db::audit::log(&pool, Some(&uid), "login", "session", None, None, None)
        .await
        .unwrap();

    let entries = db::audit::list(&pool, None, Some("repository"), 10, 0)
        .await
        .unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].action, "create_repo");
}

#[tokio::test]
async fn test_audit_list_with_pagination() {
    let pool = setup_pool().await;

    for i in 0..5 {
        db::audit::log(
            &pool,
            None,
            &format!("action_{}", i),
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

#[tokio::test]
async fn test_audit_list_filter_user_and_type() {
    let pool = setup_pool().await;
    let user = create_test_user(&pool).await;
    let uid = user.id.to_string();

    db::audit::log(&pool, Some(&uid), "push", "repository", None, None, None)
        .await
        .unwrap();
    db::audit::log(&pool, Some(&uid), "login", "session", None, None, None)
        .await
        .unwrap();

    let entries = db::audit::list(&pool, Some(&uid), Some("session"), 10, 0)
        .await
        .unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].action, "login");
}

// --- Signing key tests ---

#[tokio::test]
async fn test_signing_key_crud() {
    let pool = setup_pool().await;
    let user = create_test_user(&pool).await;
    let uid = user.id.to_string();

    let key = db::signing::add_signing_key(
        &pool,
        &uid,
        "mykey",
        "abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234",
    )
    .await
    .expect("failed to add signing key");

    assert_eq!(key.name, "mykey");
    assert_eq!(key.user_id, uid);

    // Get by ID
    let fetched = db::signing::get_signing_key(&pool, &key.id)
        .await
        .expect("failed to get key");
    assert_eq!(fetched.id, key.id);

    // List
    let keys = db::signing::list_signing_keys(&pool, &uid).await.unwrap();
    assert_eq!(keys.len(), 1);

    // Delete
    db::signing::delete_signing_key(&pool, &key.id, &uid)
        .await
        .expect("failed to delete");
    let keys = db::signing::list_signing_keys(&pool, &uid).await.unwrap();
    assert!(keys.is_empty());
}

#[tokio::test]
async fn test_signing_key_delete_wrong_user() {
    let pool = setup_pool().await;
    let user = create_test_user(&pool).await;
    let uid = user.id.to_string();

    let key = db::signing::add_signing_key(
        &pool,
        &uid,
        "mykey",
        "abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234",
    )
    .await
    .unwrap();

    let result = db::signing::delete_signing_key(&pool, &key.id, "wrong-user").await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_artifact_signature_crud() {
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
            metadata: None,
        },
    )
    .await
    .unwrap();

    let key = db::signing::add_signing_key(
        &pool,
        &user.id.to_string(),
        "mykey",
        "abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234abcd1234",
    )
    .await
    .unwrap();

    let sig = db::signing::add_signature(&pool, &artifact.id, &key.id, "deadbeef")
        .await
        .expect("failed to add signature");

    assert_eq!(sig.artifact_id, artifact.id);
    assert_eq!(sig.signer_key_id, key.id);

    let sigs = db::signing::get_signatures(&pool, &artifact.id)
        .await
        .unwrap();
    assert_eq!(sigs.len(), 1);
}

// --- SSH key tests ---

#[tokio::test]
async fn test_ssh_key_crud() {
    let pool = setup_pool().await;
    let user = create_test_user(&pool).await;
    let uid = user.id.to_string();

    let key = db::ssh_key::add(
        &pool,
        &uid,
        "laptop",
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAITest",
        "SHA256:testfingerprint123",
    )
    .await
    .expect("failed to add SSH key");

    assert_eq!(key.name, "laptop");

    // Get by ID
    let fetched = db::ssh_key::get_by_id(&pool, &key.id.to_string())
        .await
        .unwrap();
    assert_eq!(fetched.name, "laptop");

    // List
    let keys = db::ssh_key::list_by_user(&pool, &uid).await.unwrap();
    assert_eq!(keys.len(), 1);

    // Get user by fingerprint
    let result = db::ssh_key::get_user_by_fingerprint(&pool, "SHA256:testfingerprint123")
        .await
        .unwrap();
    assert!(result.is_some());
    let (found_uid, found_username) = result.unwrap();
    assert_eq!(found_uid, uid);
    assert_eq!(found_username, "testuser");

    // Delete
    db::ssh_key::delete(&pool, &key.id.to_string(), &uid)
        .await
        .unwrap();
    let keys = db::ssh_key::list_by_user(&pool, &uid).await.unwrap();
    assert!(keys.is_empty());
}

#[tokio::test]
async fn test_ssh_key_delete_wrong_user() {
    let pool = setup_pool().await;
    let user = create_test_user(&pool).await;
    let uid = user.id.to_string();

    let key = db::ssh_key::add(
        &pool,
        &uid,
        "laptop",
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAITest2",
        "SHA256:testfingerprint456",
    )
    .await
    .unwrap();

    let result = db::ssh_key::delete(&pool, &key.id.to_string(), "wrong-user").await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_ssh_key_fingerprint_not_found() {
    let pool = setup_pool().await;

    let result = db::ssh_key::get_user_by_fingerprint(&pool, "SHA256:nonexistent")
        .await
        .unwrap();
    assert!(result.is_none());
}

// --- Status check tests ---

#[tokio::test]
async fn test_status_check_upsert_and_get() {
    let pool = setup_pool().await;
    let user = create_test_user(&pool).await;
    let repo = create_test_repo(&pool, &user.id.to_string()).await;
    let repo_id = repo.id.to_string();
    let sha = "abc123def456";

    let check = db::status_check::upsert(
        &pool,
        &repo_id,
        sha,
        "ci/build",
        CheckState::Pending,
        Some("Build started"),
        Some("https://ci.example.com/1"),
    )
    .await
    .expect("failed to upsert");

    assert_eq!(check.context, "ci/build");
    assert_eq!(check.state, CheckState::Pending);

    // Update to success
    let updated = db::status_check::upsert(
        &pool,
        &repo_id,
        sha,
        "ci/build",
        CheckState::Success,
        Some("Build passed"),
        None,
    )
    .await
    .unwrap();
    assert_eq!(updated.state, CheckState::Success);

    // Should still be just 1 check (upserted)
    let checks = db::status_check::get_for_commit(&pool, &repo_id, sha)
        .await
        .unwrap();
    assert_eq!(checks.len(), 1);
}

#[tokio::test]
async fn test_status_check_all_passed() {
    let pool = setup_pool().await;
    let user = create_test_user(&pool).await;
    let repo = create_test_repo(&pool, &user.id.to_string()).await;
    let repo_id = repo.id.to_string();
    let sha = "commit123";

    // No checks = passes
    assert!(
        db::status_check::all_passed(&pool, &repo_id, sha)
            .await
            .unwrap()
    );

    // Add a passing check
    db::status_check::upsert(
        &pool,
        &repo_id,
        sha,
        "ci/build",
        CheckState::Success,
        None,
        None,
    )
    .await
    .unwrap();
    assert!(
        db::status_check::all_passed(&pool, &repo_id, sha)
            .await
            .unwrap()
    );

    // Add a failing check
    db::status_check::upsert(
        &pool,
        &repo_id,
        sha,
        "ci/lint",
        CheckState::Failure,
        None,
        None,
    )
    .await
    .unwrap();
    assert!(
        !db::status_check::all_passed(&pool, &repo_id, sha)
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn test_status_check_multiple_contexts() {
    let pool = setup_pool().await;
    let user = create_test_user(&pool).await;
    let repo = create_test_repo(&pool, &user.id.to_string()).await;
    let repo_id = repo.id.to_string();
    let sha = "commit456";

    db::status_check::upsert(
        &pool,
        &repo_id,
        sha,
        "ci/build",
        CheckState::Success,
        None,
        None,
    )
    .await
    .unwrap();
    db::status_check::upsert(
        &pool,
        &repo_id,
        sha,
        "ci/test",
        CheckState::Pending,
        None,
        None,
    )
    .await
    .unwrap();
    db::status_check::upsert(
        &pool,
        &repo_id,
        sha,
        "ci/lint",
        CheckState::Error,
        None,
        None,
    )
    .await
    .unwrap();

    let checks = db::status_check::get_for_commit(&pool, &repo_id, sha)
        .await
        .unwrap();
    assert_eq!(checks.len(), 3);

    // Not all passed (pending + error)
    assert!(
        !db::status_check::all_passed(&pool, &repo_id, sha)
            .await
            .unwrap()
    );
}

// --- LFS tests ---

#[tokio::test]
async fn test_lfs_create_and_get() {
    let pool = setup_pool().await;
    let user = create_test_user(&pool).await;
    let repo = create_test_repo(&pool, &user.id.to_string()).await;
    let repo_id = repo.id.to_string();

    let obj = db::lfs::create(&pool, &repo_id, "sha256:abc123", 1024)
        .await
        .expect("failed to create LFS object");

    assert_eq!(obj.oid, "sha256:abc123");
    assert_eq!(obj.size, 1024);

    let fetched = db::lfs::get(&pool, &repo_id, "sha256:abc123")
        .await
        .unwrap();
    assert_eq!(fetched.id, obj.id);
}

#[tokio::test]
async fn test_lfs_exists() {
    let pool = setup_pool().await;
    let user = create_test_user(&pool).await;
    let repo = create_test_repo(&pool, &user.id.to_string()).await;
    let repo_id = repo.id.to_string();

    assert!(
        !db::lfs::exists(&pool, &repo_id, "sha256:nope")
            .await
            .unwrap()
    );

    db::lfs::create(&pool, &repo_id, "sha256:exists123", 512)
        .await
        .unwrap();

    assert!(
        db::lfs::exists(&pool, &repo_id, "sha256:exists123")
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn test_lfs_list_and_delete() {
    let pool = setup_pool().await;
    let user = create_test_user(&pool).await;
    let repo = create_test_repo(&pool, &user.id.to_string()).await;
    let repo_id = repo.id.to_string();

    db::lfs::create(&pool, &repo_id, "sha256:obj1", 100)
        .await
        .unwrap();
    db::lfs::create(&pool, &repo_id, "sha256:obj2", 200)
        .await
        .unwrap();

    let list = db::lfs::list_by_repo(&pool, &repo_id).await.unwrap();
    assert_eq!(list.len(), 2);

    db::lfs::delete(&pool, &repo_id, "sha256:obj1")
        .await
        .unwrap();

    let list = db::lfs::list_by_repo(&pool, &repo_id).await.unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].oid, "sha256:obj2");
}

// --- Retention tests ---

#[tokio::test]
async fn test_retention_policy_set_and_get() {
    let pool = setup_pool().await;
    let user = create_test_user(&pool).await;
    let repo = create_test_repo(&pool, &user.id.to_string()).await;
    let repo_id = repo.id.to_string();

    // No policy initially
    let policy = db::retention::get_policy(&pool, &repo_id).await.unwrap();
    assert!(policy.is_none());

    // Set policy
    let policy = db::retention::set_policy(
        &pool,
        &db::retention::SetPolicyParams {
            repo_id: &repo_id,
            max_age_days: Some(30),
            max_count: Some(100),
            max_total_bytes: Some(1_000_000),
        },
    )
    .await
    .expect("failed to set policy");

    assert_eq!(policy.max_age_days, Some(30));
    assert_eq!(policy.max_count, Some(100));
    assert_eq!(policy.max_total_bytes, Some(1_000_000));

    // Update policy
    let updated = db::retention::set_policy(
        &pool,
        &db::retention::SetPolicyParams {
            repo_id: &repo_id,
            max_age_days: Some(7),
            max_count: None,
            max_total_bytes: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(updated.max_age_days, Some(7));
    assert!(updated.max_count.is_none());
}

#[tokio::test]
async fn test_retention_total_artifact_size() {
    let pool = setup_pool().await;
    let user = create_test_user(&pool).await;
    let repo = create_test_repo(&pool, &user.id.to_string()).await;
    let repo_id = repo.id.to_string();

    // Empty repo = 0
    let total = db::retention::total_artifact_size(&pool, &repo_id)
        .await
        .unwrap();
    assert_eq!(total, 0);

    db::artifact::create(
        &pool,
        &db::artifact::CreateArtifactParams {
            repo_id: &repo_id,
            pipeline_id: None,
            name: "a.tar",
            version: None,
            artifact_type: "generic",
            content_hash: "hash1",
            size_bytes: 500,
            metadata: None,
        },
    )
    .await
    .unwrap();

    db::artifact::create(
        &pool,
        &db::artifact::CreateArtifactParams {
            repo_id: &repo_id,
            pipeline_id: None,
            name: "b.tar",
            version: None,
            artifact_type: "generic",
            content_hash: "hash2",
            size_bytes: 300,
            metadata: None,
        },
    )
    .await
    .unwrap();

    let total = db::retention::total_artifact_size(&pool, &repo_id)
        .await
        .unwrap();
    assert_eq!(total, 800);
}

#[tokio::test]
async fn test_retention_find_excess_artifacts() {
    let pool = setup_pool().await;
    let user = create_test_user(&pool).await;
    let repo = create_test_repo(&pool, &user.id.to_string()).await;
    let repo_id = repo.id.to_string();

    for i in 0..5 {
        db::artifact::create(
            &pool,
            &db::artifact::CreateArtifactParams {
                repo_id: &repo_id,
                pipeline_id: None,
                name: &format!("artifact_{}", i),
                version: None,
                artifact_type: "generic",
                content_hash: &format!("hash_{}", i),
                size_bytes: 100,
                metadata: None,
            },
        )
        .await
        .unwrap();
    }

    // Keep 3 → should mark 2 for deletion
    let excess = db::retention::find_excess_artifacts(&pool, &repo_id, 3)
        .await
        .unwrap();
    assert_eq!(excess.len(), 2);
}

#[tokio::test]
async fn test_retention_find_oversize_artifacts() {
    let pool = setup_pool().await;
    let user = create_test_user(&pool).await;
    let repo = create_test_repo(&pool, &user.id.to_string()).await;
    let repo_id = repo.id.to_string();

    for i in 0..4 {
        db::artifact::create(
            &pool,
            &db::artifact::CreateArtifactParams {
                repo_id: &repo_id,
                pipeline_id: None,
                name: &format!("art_{}", i),
                version: None,
                artifact_type: "generic",
                content_hash: &format!("h_{}", i),
                size_bytes: 250,
                metadata: None,
            },
        )
        .await
        .unwrap();
    }

    // Total is 1000, limit to 500 → need to free 500
    let oversize = db::retention::find_oversize_artifacts(&pool, &repo_id, 500)
        .await
        .unwrap();
    assert_eq!(oversize.len(), 2); // 2 × 250 = 500 freed

    // Under limit → no deletions
    let oversize = db::retention::find_oversize_artifacts(&pool, &repo_id, 2000)
        .await
        .unwrap();
    assert!(oversize.is_empty());
}

// --- Ark package tests ---

#[tokio::test]
async fn test_ark_package_publish_and_get() {
    let pool = setup_pool().await;
    let user = create_test_user(&pool).await;
    let repo = create_test_repo(&pool, &user.id.to_string()).await;
    let repo_id = repo.id.to_string();
    let uid = user.id.to_string();

    // Create artifact first
    let artifact = db::artifact::create(
        &pool,
        &db::artifact::CreateArtifactParams {
            repo_id: &repo_id,
            pipeline_id: None,
            name: "mylib",
            version: Some("1.0.0"),
            artifact_type: "ark",
            content_hash: "arkhash1",
            size_bytes: 2048,
            metadata: None,
        },
    )
    .await
    .unwrap();

    let pkg = db::ark_package::publish(
        &pool,
        &db::ark_package::PublishParams {
            artifact_id: &artifact.id,
            repo_id: &repo_id,
            publisher_id: &uid,
            package_name: "mylib",
            version: "1.0.0",
            arch: "x86_64",
            description: Some("A test library"),
            dependencies: Some("libc>=2.0"),
            provides: None,
        },
    )
    .await
    .expect("failed to publish");

    assert_eq!(pkg.package_name, "mylib");
    assert_eq!(pkg.version, "1.0.0");
    assert_eq!(pkg.arch, "x86_64");

    // Get by ID
    let fetched = db::ark_package::get(&pool, &pkg.id).await.unwrap();
    assert_eq!(fetched.package_name, "mylib");
}

#[tokio::test]
async fn test_ark_package_list_versions() {
    let pool = setup_pool().await;
    let user = create_test_user(&pool).await;
    let repo = create_test_repo(&pool, &user.id.to_string()).await;
    let repo_id = repo.id.to_string();
    let uid = user.id.to_string();

    for ver in ["1.0.0", "1.1.0", "2.0.0"] {
        let artifact = db::artifact::create(
            &pool,
            &db::artifact::CreateArtifactParams {
                repo_id: &repo_id,
                pipeline_id: None,
                name: &format!("pkg-{}", ver),
                version: Some(ver),
                artifact_type: "ark",
                content_hash: &format!("hash-{}", ver),
                size_bytes: 100,
                metadata: None,
            },
        )
        .await
        .unwrap();

        db::ark_package::publish(
            &pool,
            &db::ark_package::PublishParams {
                artifact_id: &artifact.id,
                repo_id: &repo_id,
                publisher_id: &uid,
                package_name: "mypkg",
                version: ver,
                arch: "any",
                description: None,
                dependencies: None,
                provides: None,
            },
        )
        .await
        .unwrap();
    }

    let versions = db::ark_package::list_versions(&pool, "mypkg")
        .await
        .unwrap();
    assert_eq!(versions.len(), 3);
}

#[tokio::test]
async fn test_ark_package_get_version() {
    let pool = setup_pool().await;
    let user = create_test_user(&pool).await;
    let repo = create_test_repo(&pool, &user.id.to_string()).await;
    let repo_id = repo.id.to_string();
    let uid = user.id.to_string();

    let artifact = db::artifact::create(
        &pool,
        &db::artifact::CreateArtifactParams {
            repo_id: &repo_id,
            pipeline_id: None,
            name: "pkg-v1",
            version: Some("1.0.0"),
            artifact_type: "ark",
            content_hash: "vhash1",
            size_bytes: 100,
            metadata: None,
        },
    )
    .await
    .unwrap();

    db::ark_package::publish(
        &pool,
        &db::ark_package::PublishParams {
            artifact_id: &artifact.id,
            repo_id: &repo_id,
            publisher_id: &uid,
            package_name: "versioned",
            version: "1.0.0",
            arch: "any",
            description: None,
            dependencies: None,
            provides: None,
        },
    )
    .await
    .unwrap();

    let pkg = db::ark_package::get_version(&pool, "versioned", "1.0.0", None)
        .await
        .unwrap();
    assert_eq!(pkg.version, "1.0.0");

    // Not found
    let result = db::ark_package::get_version(&pool, "versioned", "9.9.9", None).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_ark_package_search() {
    let pool = setup_pool().await;
    let user = create_test_user(&pool).await;
    let repo = create_test_repo(&pool, &user.id.to_string()).await;
    let repo_id = repo.id.to_string();
    let uid = user.id.to_string();

    let artifact = db::artifact::create(
        &pool,
        &db::artifact::CreateArtifactParams {
            repo_id: &repo_id,
            pipeline_id: None,
            name: "searchpkg",
            version: Some("1.0.0"),
            artifact_type: "ark",
            content_hash: "shash1",
            size_bytes: 100,
            metadata: None,
        },
    )
    .await
    .unwrap();

    db::ark_package::publish(
        &pool,
        &db::ark_package::PublishParams {
            artifact_id: &artifact.id,
            repo_id: &repo_id,
            publisher_id: &uid,
            package_name: "awesome-lib",
            version: "1.0.0",
            arch: "any",
            description: Some("An awesome library for testing"),
            dependencies: None,
            provides: None,
        },
    )
    .await
    .unwrap();

    // Search by name
    let results = db::ark_package::search(&pool, "awesome", 10, 0)
        .await
        .unwrap();
    assert_eq!(results.len(), 1);

    // Search by description
    let results = db::ark_package::search(&pool, "testing", 10, 0)
        .await
        .unwrap();
    assert_eq!(results.len(), 1);

    // No results
    let results = db::ark_package::search(&pool, "nonexistent", 10, 0)
        .await
        .unwrap();
    assert!(results.is_empty());
}

#[tokio::test]
async fn test_ark_package_delete() {
    let pool = setup_pool().await;
    let user = create_test_user(&pool).await;
    let repo = create_test_repo(&pool, &user.id.to_string()).await;
    let repo_id = repo.id.to_string();
    let uid = user.id.to_string();

    let artifact = db::artifact::create(
        &pool,
        &db::artifact::CreateArtifactParams {
            repo_id: &repo_id,
            pipeline_id: None,
            name: "delpkg",
            version: Some("1.0.0"),
            artifact_type: "ark",
            content_hash: "dhash1",
            size_bytes: 100,
            metadata: None,
        },
    )
    .await
    .unwrap();

    let pkg = db::ark_package::publish(
        &pool,
        &db::ark_package::PublishParams {
            artifact_id: &artifact.id,
            repo_id: &repo_id,
            publisher_id: &uid,
            package_name: "deleteme",
            version: "1.0.0",
            arch: "any",
            description: None,
            dependencies: None,
            provides: None,
        },
    )
    .await
    .unwrap();

    db::ark_package::delete(&pool, &pkg.id)
        .await
        .expect("delete failed");

    let result = db::ark_package::get(&pool, &pkg.id).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_ark_package_get_latest() {
    let pool = setup_pool().await;
    let user = create_test_user(&pool).await;
    let repo = create_test_repo(&pool, &user.id.to_string()).await;
    let repo_id = repo.id.to_string();
    let uid = user.id.to_string();

    for (ver, arch) in [
        ("1.0.0", "x86_64"),
        ("1.1.0", "x86_64"),
        ("1.0.0", "aarch64"),
    ] {
        let artifact = db::artifact::create(
            &pool,
            &db::artifact::CreateArtifactParams {
                repo_id: &repo_id,
                pipeline_id: None,
                name: &format!("latestpkg-{}-{}", ver, arch),
                version: Some(ver),
                artifact_type: "ark",
                content_hash: &format!("lhash-{}-{}", ver, arch),
                size_bytes: 100,
                metadata: None,
            },
        )
        .await
        .unwrap();

        db::ark_package::publish(
            &pool,
            &db::ark_package::PublishParams {
                artifact_id: &artifact.id,
                repo_id: &repo_id,
                publisher_id: &uid,
                package_name: "latestpkg",
                version: ver,
                arch,
                description: None,
                dependencies: None,
                provides: None,
            },
        )
        .await
        .unwrap();
    }

    // Latest overall
    let latest = db::ark_package::get_latest(&pool, "latestpkg", None)
        .await
        .unwrap();
    assert_eq!(latest.package_name, "latestpkg");

    // Latest for specific arch
    let latest = db::ark_package::get_latest(&pool, "latestpkg", Some("aarch64"))
        .await
        .unwrap();
    assert_eq!(latest.arch, "aarch64");

    // Not found
    let result = db::ark_package::get_latest(&pool, "nonexistent", None).await;
    assert!(result.is_err());
}
