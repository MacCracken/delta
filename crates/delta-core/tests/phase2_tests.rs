use delta_core::db;
use delta_core::models::branch_protection::BranchProtection;
use delta_core::models::repo::Visibility;
use uuid::Uuid;

async fn setup_pool() -> sqlx::SqlitePool {
    let pool = sqlx::SqlitePool::connect("sqlite::memory:")
        .await
        .expect("failed to connect to in-memory db");

    for migration in [
        include_str!("../migrations/001_initial.sql"),
        include_str!("../migrations/002_git_protocol.sql"),
    ] {
        sqlx::query(migration)
            .execute(&pool)
            .await
            .expect("failed to run migration");
    }

    pool
}

async fn create_test_user(pool: &sqlx::SqlitePool) -> (String, String) {
    let user = db::user::create(pool, "testuser", "test@example.com", "hashedpw", false)
        .await
        .unwrap();
    (user.id.to_string(), user.username)
}

async fn create_test_repo(pool: &sqlx::SqlitePool, owner_id: &str) -> String {
    let repo = db::repo::create(pool, owner_id, "testrepo", None, Visibility::Private)
        .await
        .unwrap();
    repo.id.to_string()
}

// --- Branch Protection Tests ---

#[test]
fn test_branch_protection_matching() {
    let p = BranchProtection {
        id: Uuid::new_v4(),
        repo_id: Uuid::new_v4(),
        pattern: "main".to_string(),
        require_pr: true,
        required_approvals: 1,
        require_status_checks: true,
        prevent_force_push: true,
        prevent_deletion: true,
    };

    assert!(p.matches("main"));
    assert!(!p.matches("develop"));
    assert!(!p.matches("main/something"));
}

#[test]
fn test_branch_protection_glob_matching() {
    let p = BranchProtection {
        id: Uuid::new_v4(),
        repo_id: Uuid::new_v4(),
        pattern: "release/*".to_string(),
        require_pr: false,
        required_approvals: 0,
        require_status_checks: false,
        prevent_force_push: true,
        prevent_deletion: true,
    };

    assert!(p.matches("release/2026.1.1"));
    assert!(p.matches("release/2026.3"));
    assert!(!p.matches("release"));
    assert!(!p.matches("main"));
}

#[test]
fn test_branch_protection_direct_push() {
    let mut p = BranchProtection {
        id: Uuid::new_v4(),
        repo_id: Uuid::new_v4(),
        pattern: "main".to_string(),
        require_pr: false,
        required_approvals: 0,
        require_status_checks: false,
        prevent_force_push: false,
        prevent_deletion: false,
    };

    assert!(p.allows_direct_push());
    assert!(p.allows_force_push());

    p.require_pr = true;
    p.prevent_force_push = true;
    assert!(!p.allows_direct_push());
    assert!(!p.allows_force_push());
}

#[tokio::test]
async fn test_branch_protection_crud() {
    let pool = setup_pool().await;
    let (uid, _) = create_test_user(&pool).await;
    let repo_id = create_test_repo(&pool, &uid).await;

    // Create
    let protection = db::branch_protection::create(
        &pool,
        db::branch_protection::CreateParams {
            repo_id: &repo_id,
            pattern: "main",
            require_pr: true,
            required_approvals: 2,
            require_status_checks: true,
            prevent_force_push: true,
            prevent_deletion: true,
        },
    )
    .await
    .unwrap();

    assert_eq!(protection.pattern, "main");
    assert_eq!(protection.required_approvals, 2);

    // List
    let protections = db::branch_protection::list_for_repo(&pool, &repo_id)
        .await
        .unwrap();
    assert_eq!(protections.len(), 1);

    // Find matching
    let found = db::branch_protection::find_matching(&pool, &repo_id, "main")
        .await
        .unwrap();
    assert!(found.is_some());
    assert_eq!(found.unwrap().pattern, "main");

    let not_found = db::branch_protection::find_matching(&pool, &repo_id, "develop")
        .await
        .unwrap();
    assert!(not_found.is_none());

    // Delete
    db::branch_protection::delete(&pool, &protection.id.to_string())
        .await
        .unwrap();

    let protections = db::branch_protection::list_for_repo(&pool, &repo_id)
        .await
        .unwrap();
    assert_eq!(protections.len(), 0);
}

#[tokio::test]
async fn test_duplicate_protection_pattern_fails() {
    let pool = setup_pool().await;
    let (uid, _) = create_test_user(&pool).await;
    let repo_id = create_test_repo(&pool, &uid).await;

    db::branch_protection::create(
        &pool,
        db::branch_protection::CreateParams {
            repo_id: &repo_id,
            pattern: "main",
            require_pr: false,
            required_approvals: 0,
            require_status_checks: false,
            prevent_force_push: true,
            prevent_deletion: true,
        },
    )
    .await
    .unwrap();

    let result = db::branch_protection::create(
        &pool,
        db::branch_protection::CreateParams {
            repo_id: &repo_id,
            pattern: "main",
            require_pr: true,
            required_approvals: 1,
            require_status_checks: false,
            prevent_force_push: true,
            prevent_deletion: true,
        },
    )
    .await;
    assert!(result.is_err());
}

// --- Webhook Tests ---

#[tokio::test]
async fn test_webhook_crud() {
    let pool = setup_pool().await;
    let (uid, _) = create_test_user(&pool).await;
    let repo_id = create_test_repo(&pool, &uid).await;

    // Create
    let id = db::webhook::create(
        &pool,
        &repo_id,
        "https://example.com/webhook",
        Some("mysecret"),
        r#"["push","tag_create"]"#,
    )
    .await
    .unwrap();

    // List
    let webhooks = db::webhook::list_for_repo(&pool, &repo_id).await.unwrap();
    assert_eq!(webhooks.len(), 1);
    assert_eq!(webhooks[0].url, "https://example.com/webhook");

    // Get for event
    let push_hooks = db::webhook::get_for_event(&pool, &repo_id, "push")
        .await
        .unwrap();
    assert_eq!(push_hooks.len(), 1);

    let pr_hooks = db::webhook::get_for_event(&pool, &repo_id, "pull_request")
        .await
        .unwrap();
    assert_eq!(pr_hooks.len(), 0);

    // Delete
    db::webhook::delete(&pool, &id, &repo_id).await.unwrap();

    let webhooks = db::webhook::list_for_repo(&pool, &repo_id).await.unwrap();
    assert_eq!(webhooks.len(), 0);
}

#[tokio::test]
async fn test_webhook_delivery_recording() {
    let pool = setup_pool().await;
    let (uid, _) = create_test_user(&pool).await;
    let repo_id = create_test_repo(&pool, &uid).await;

    let webhook_id = db::webhook::create(
        &pool,
        &repo_id,
        "https://example.com/hook",
        None,
        r#"["push"]"#,
    )
    .await
    .unwrap();

    db::webhook::record_delivery(
        &pool,
        &webhook_id,
        "push",
        r#"{"ref":"refs/heads/main"}"#,
        Some(200),
        Some("OK"),
    )
    .await
    .unwrap();

    // Verify delivery was recorded (query directly)
    let count: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM webhook_deliveries WHERE webhook_id = ?")
            .bind(&webhook_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(count.0, 1);
}
