//! Phase 8/9 tests: federation, encryption, audit export, crypto repo keys.

use delta_core::db;

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

async fn create_test_repo(
    pool: &sqlx::SqlitePool,
    owner_id: &str,
) -> delta_core::models::repo::Repository {
    db::repo::create(
        pool,
        owner_id,
        "testrepo",
        Some("desc"),
        delta_core::models::repo::Visibility::Public,
    )
    .await
    .expect("failed to create repo")
}

// --- Federation tests ---

#[tokio::test]
async fn test_federation_add_instance() {
    let pool = setup_pool().await;
    let inst = db::federation::add_instance(
        &pool,
        "https://remote.example.com",
        Some("Remote Delta"),
        Some("abcd1234"),
        false,
    )
    .await
    .unwrap();

    assert_eq!(inst.url, "https://remote.example.com");
    assert_eq!(inst.name.as_deref(), Some("Remote Delta"));
    assert_eq!(inst.public_key.as_deref(), Some("abcd1234"));
    assert!(!inst.trusted);
    assert!(inst.last_seen_at.is_none());
}

#[tokio::test]
async fn test_federation_list_instances() {
    let pool = setup_pool().await;
    db::federation::add_instance(&pool, "https://a.example.com", None, None, false)
        .await
        .unwrap();
    db::federation::add_instance(&pool, "https://b.example.com", None, None, true)
        .await
        .unwrap();

    let list = db::federation::list_instances(&pool).await.unwrap();
    assert_eq!(list.len(), 2);
}

#[tokio::test]
async fn test_federation_get_instance() {
    let pool = setup_pool().await;
    let inst = db::federation::add_instance(&pool, "https://c.example.com", None, None, false)
        .await
        .unwrap();

    let fetched = db::federation::get_instance(&pool, &inst.id).await.unwrap();
    assert_eq!(fetched.url, "https://c.example.com");
}

#[tokio::test]
async fn test_federation_get_instance_by_url() {
    let pool = setup_pool().await;
    db::federation::add_instance(&pool, "https://d.example.com", Some("D"), None, false)
        .await
        .unwrap();

    let fetched = db::federation::get_instance_by_url(&pool, "https://d.example.com")
        .await
        .unwrap();
    assert_eq!(fetched.name.as_deref(), Some("D"));
}

#[tokio::test]
async fn test_federation_update_trust() {
    let pool = setup_pool().await;
    let inst = db::federation::add_instance(&pool, "https://e.example.com", None, None, false)
        .await
        .unwrap();
    assert!(!inst.trusted);

    db::federation::update_trust(&pool, &inst.id, true)
        .await
        .unwrap();
    let updated = db::federation::get_instance(&pool, &inst.id).await.unwrap();
    assert!(updated.trusted);
}

#[tokio::test]
async fn test_federation_update_last_seen() {
    let pool = setup_pool().await;
    let inst = db::federation::add_instance(&pool, "https://f.example.com", None, None, false)
        .await
        .unwrap();
    assert!(inst.last_seen_at.is_none());

    db::federation::update_last_seen(&pool, &inst.id)
        .await
        .unwrap();
    let updated = db::federation::get_instance(&pool, &inst.id).await.unwrap();
    assert!(updated.last_seen_at.is_some());
}

#[tokio::test]
async fn test_federation_delete_instance() {
    let pool = setup_pool().await;
    let inst = db::federation::add_instance(&pool, "https://g.example.com", None, None, false)
        .await
        .unwrap();

    db::federation::delete_instance(&pool, &inst.id)
        .await
        .unwrap();
    let list = db::federation::list_instances(&pool).await.unwrap();
    assert!(list.is_empty());
}

#[tokio::test]
async fn test_federation_duplicate_url_rejected() {
    let pool = setup_pool().await;
    db::federation::add_instance(&pool, "https://h.example.com", None, None, false)
        .await
        .unwrap();

    let result =
        db::federation::add_instance(&pool, "https://h.example.com", None, None, false).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_federation_delete_nonexistent() {
    let pool = setup_pool().await;
    let result = db::federation::delete_instance(&pool, "nonexistent-id").await;
    assert!(result.is_err());
}

// --- Encryption key tests ---

#[tokio::test]
async fn test_encryption_add_and_get_key() {
    let pool = setup_pool().await;
    let user = create_test_user(&pool).await;
    let repo = create_test_repo(&pool, &user.id.to_string()).await;

    let key = db::encryption::add_key(
        &pool,
        &repo.id.to_string(),
        &user.id.to_string(),
        "encrypted_key_data_hex",
        "xchacha20-poly1305",
    )
    .await
    .unwrap();

    assert_eq!(key.repo_id, repo.id.to_string());
    assert_eq!(key.user_id, user.id.to_string());
    assert_eq!(key.encrypted_key, "encrypted_key_data_hex");
    assert_eq!(key.algorithm, "xchacha20-poly1305");

    let fetched = db::encryption::get_key(&pool, &repo.id.to_string(), &user.id.to_string())
        .await
        .unwrap();
    assert_eq!(fetched.id, key.id);
}

#[tokio::test]
async fn test_encryption_list_keys_for_repo() {
    let pool = setup_pool().await;
    let user1 = create_test_user(&pool).await;
    let user2 = db::user::create(&pool, "user2", "u2@test.com", "pw", false)
        .await
        .unwrap();
    let repo = create_test_repo(&pool, &user1.id.to_string()).await;

    db::encryption::add_key(
        &pool,
        &repo.id.to_string(),
        &user1.id.to_string(),
        "key1",
        "xchacha20-poly1305",
    )
    .await
    .unwrap();
    db::encryption::add_key(
        &pool,
        &repo.id.to_string(),
        &user2.id.to_string(),
        "key2",
        "xchacha20-poly1305",
    )
    .await
    .unwrap();

    let keys = db::encryption::list_keys_for_repo(&pool, &repo.id.to_string())
        .await
        .unwrap();
    assert_eq!(keys.len(), 2);
}

#[tokio::test]
async fn test_encryption_delete_key() {
    let pool = setup_pool().await;
    let user = create_test_user(&pool).await;
    let repo = create_test_repo(&pool, &user.id.to_string()).await;

    let key = db::encryption::add_key(
        &pool,
        &repo.id.to_string(),
        &user.id.to_string(),
        "key_data",
        "xchacha20-poly1305",
    )
    .await
    .unwrap();

    db::encryption::delete_key(&pool, &key.id).await.unwrap();
    let keys = db::encryption::list_keys_for_repo(&pool, &repo.id.to_string())
        .await
        .unwrap();
    assert!(keys.is_empty());
}

#[tokio::test]
async fn test_encryption_duplicate_user_repo_rejected() {
    let pool = setup_pool().await;
    let user = create_test_user(&pool).await;
    let repo = create_test_repo(&pool, &user.id.to_string()).await;

    db::encryption::add_key(
        &pool,
        &repo.id.to_string(),
        &user.id.to_string(),
        "key1",
        "xchacha20-poly1305",
    )
    .await
    .unwrap();

    let result = db::encryption::add_key(
        &pool,
        &repo.id.to_string(),
        &user.id.to_string(),
        "key2",
        "xchacha20-poly1305",
    )
    .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_encryption_set_repo_encrypted() {
    let pool = setup_pool().await;
    let user = create_test_user(&pool).await;
    let repo = create_test_repo(&pool, &user.id.to_string()).await;

    // Should succeed without error
    db::encryption::set_repo_encrypted(&pool, &repo.id.to_string(), true)
        .await
        .unwrap();

    // Verify via raw query that the column was updated
    let encrypted: bool = sqlx::query_scalar("SELECT encrypted FROM repositories WHERE id = ?")
        .bind(&repo.id.to_string())
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(encrypted);
}

// --- Audit export tests ---

#[tokio::test]
async fn test_audit_list_for_export() {
    let pool = setup_pool().await;
    let user = create_test_user(&pool).await;
    let uid = user.id.to_string();

    // Create some audit entries
    for action in ["create", "update", "delete"] {
        db::audit::log(&pool, Some(&uid), action, "repo", Some("repo1"), None, None)
            .await
            .unwrap();
    }

    let entries = db::audit::list_for_export(&pool, None, None, None, 100, 0)
        .await
        .unwrap();
    assert_eq!(entries.len(), 3);

    // Filter by resource type
    let entries = db::audit::list_for_export(&pool, None, None, Some("repo"), 100, 0)
        .await
        .unwrap();
    assert_eq!(entries.len(), 3);

    // Filter by non-existent type
    let entries = db::audit::list_for_export(&pool, None, None, Some("nonexistent"), 100, 0)
        .await
        .unwrap();
    assert!(entries.is_empty());
}

#[tokio::test]
async fn test_audit_count_for_export() {
    let pool = setup_pool().await;
    let user = create_test_user(&pool).await;
    let uid = user.id.to_string();

    for _ in 0..5 {
        db::audit::log(&pool, Some(&uid), "push", "repo", Some("r1"), None, None)
            .await
            .unwrap();
    }

    let count = db::audit::count_for_export(&pool, None, None, None)
        .await
        .unwrap();
    assert_eq!(count, 5);
}

#[tokio::test]
async fn test_audit_export_pagination() {
    let pool = setup_pool().await;
    let user = create_test_user(&pool).await;
    let uid = user.id.to_string();

    for i in 0..10 {
        db::audit::log(
            &pool,
            Some(&uid),
            &format!("action_{}", i),
            "repo",
            None,
            None,
            None,
        )
        .await
        .unwrap();
    }

    let page1 = db::audit::list_for_export(&pool, None, None, None, 3, 0)
        .await
        .unwrap();
    assert_eq!(page1.len(), 3);

    let page2 = db::audit::list_for_export(&pool, None, None, None, 3, 3)
        .await
        .unwrap();
    assert_eq!(page2.len(), 3);

    // Different entries
    assert_ne!(page1[0].id, page2[0].id);
}

// --- Mirror repo tests ---

#[tokio::test]
async fn test_create_mirror_repo() {
    let pool = setup_pool().await;
    let user = create_test_user(&pool).await;
    let uid = user.id.to_string();

    let inst = db::federation::add_instance(&pool, "https://remote.test", None, None, true)
        .await
        .unwrap();

    let mirror = db::repo::create_mirror(
        &pool,
        &uid,
        "mirror-repo",
        Some("A mirror"),
        "https://remote.test/foo/bar.git",
        Some(&inst.id),
    )
    .await
    .unwrap();

    assert_eq!(mirror.name, "mirror-repo");
    assert!(mirror.is_mirror);
    assert_eq!(
        mirror.mirror_url.as_deref(),
        Some("https://remote.test/foo/bar.git")
    );
    assert_eq!(
        mirror.federation_instance_id.as_deref(),
        Some(inst.id.as_str())
    );
}

// --- Crypto repo key tests ---

#[test]
fn test_generate_repo_key() {
    let key = delta_core::crypto::generate_repo_key();
    assert_eq!(key.len(), 64); // 32 bytes as hex
    assert!(key.chars().all(|c| c.is_ascii_hexdigit()));

    // Each call produces a different key
    let key2 = delta_core::crypto::generate_repo_key();
    assert_ne!(key, key2);
}

#[test]
fn test_wrap_unwrap_repo_key() {
    let user_key = delta_core::crypto::derive_key("user-password");
    let repo_key = delta_core::crypto::generate_repo_key();

    let wrapped = delta_core::crypto::wrap_repo_key(&user_key, &repo_key);
    assert_ne!(wrapped, repo_key); // Wrapped != original

    let unwrapped = delta_core::crypto::unwrap_repo_key(&user_key, &wrapped).unwrap();
    assert_eq!(unwrapped, repo_key);
}

#[test]
fn test_wrap_wrong_key_fails() {
    let key1 = delta_core::crypto::derive_key("user1");
    let key2 = delta_core::crypto::derive_key("user2");
    let repo_key = delta_core::crypto::generate_repo_key();

    let wrapped = delta_core::crypto::wrap_repo_key(&key1, &repo_key);
    let unwrapped = delta_core::crypto::unwrap_repo_key(&key2, &wrapped).unwrap_or_default();
    assert_ne!(unwrapped, repo_key);
}
