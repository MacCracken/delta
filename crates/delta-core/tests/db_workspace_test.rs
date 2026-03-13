//! Tests for workspace CRUD operations.

mod common;

use delta_core::db;
use delta_core::models::workspace::WorkspaceStatus;

#[tokio::test]
async fn test_create_and_get_workspace() {
    let pool = common::setup_pool().await;
    let user = common::create_test_user(&pool).await;
    let repo = common::create_test_repo(&pool, &user.id.to_string()).await;

    let ws = db::workspace::create(
        &pool,
        db::workspace::CreateWorkspaceParams {
            repo_id: &repo.id.to_string(),
            creator_id: &user.id.to_string(),
            name: "fix-auth",
            branch: "ws/abc123/fix-auth",
            base_branch: "main",
            base_commit: "deadbeef",
            ttl_hours: 24,
        },
    )
    .await
    .unwrap();

    assert_eq!(ws.name, "fix-auth");
    assert_eq!(ws.branch, "ws/abc123/fix-auth");
    assert_eq!(ws.base_branch, "main");
    assert_eq!(ws.base_commit, "deadbeef");
    assert_eq!(ws.status, WorkspaceStatus::Active);
    assert!(ws.head_commit.is_none());

    let fetched = db::workspace::get_by_id(&pool, &ws.id.to_string())
        .await
        .unwrap();
    assert_eq!(fetched.id, ws.id);
    assert_eq!(fetched.name, "fix-auth");
}

#[tokio::test]
async fn test_list_workspaces() {
    let pool = common::setup_pool().await;
    let user = common::create_test_user(&pool).await;
    let repo = common::create_test_repo(&pool, &user.id.to_string()).await;
    let repo_id = repo.id.to_string();

    for i in 0..3 {
        db::workspace::create(
            &pool,
            db::workspace::CreateWorkspaceParams {
                repo_id: &repo_id,
                creator_id: &user.id.to_string(),
                name: &format!("ws-{}", i),
                branch: &format!("ws/id{}/ws-{}", i, i),
                base_branch: "main",
                base_commit: "abc123",
                ttl_hours: 24,
            },
        )
        .await
        .unwrap();
    }

    let all = db::workspace::list_for_repo(&pool, &repo_id, None, 50)
        .await
        .unwrap();
    assert_eq!(all.len(), 3);

    let active = db::workspace::list_for_repo(&pool, &repo_id, Some("active"), 50)
        .await
        .unwrap();
    assert_eq!(active.len(), 3);
}

#[tokio::test]
async fn test_update_head_commit() {
    let pool = common::setup_pool().await;
    let user = common::create_test_user(&pool).await;
    let repo = common::create_test_repo(&pool, &user.id.to_string()).await;

    let ws = db::workspace::create(
        &pool,
        db::workspace::CreateWorkspaceParams {
            repo_id: &repo.id.to_string(),
            creator_id: &user.id.to_string(),
            name: "test-ws",
            branch: "ws/xyz/test-ws",
            base_branch: "main",
            base_commit: "abc",
            ttl_hours: 12,
        },
    )
    .await
    .unwrap();

    assert!(ws.head_commit.is_none());

    let updated = db::workspace::update_head_commit(&pool, &ws.id.to_string(), "newsha123")
        .await
        .unwrap();
    assert_eq!(updated.head_commit.as_deref(), Some("newsha123"));
}

#[tokio::test]
async fn test_update_status() {
    let pool = common::setup_pool().await;
    let user = common::create_test_user(&pool).await;
    let repo = common::create_test_repo(&pool, &user.id.to_string()).await;

    let ws = db::workspace::create(
        &pool,
        db::workspace::CreateWorkspaceParams {
            repo_id: &repo.id.to_string(),
            creator_id: &user.id.to_string(),
            name: "close-me",
            branch: "ws/aaa/close-me",
            base_branch: "main",
            base_commit: "abc",
            ttl_hours: 24,
        },
    )
    .await
    .unwrap();

    assert_eq!(ws.status, WorkspaceStatus::Active);

    let closed = db::workspace::update_status(&pool, &ws.id.to_string(), WorkspaceStatus::Closed)
        .await
        .unwrap();
    assert_eq!(closed.status, WorkspaceStatus::Closed);
}

#[tokio::test]
async fn test_extend_ttl() {
    let pool = common::setup_pool().await;
    let user = common::create_test_user(&pool).await;
    let repo = common::create_test_repo(&pool, &user.id.to_string()).await;

    let ws = db::workspace::create(
        &pool,
        db::workspace::CreateWorkspaceParams {
            repo_id: &repo.id.to_string(),
            creator_id: &user.id.to_string(),
            name: "extend-me",
            branch: "ws/bbb/extend-me",
            base_branch: "main",
            base_commit: "abc",
            ttl_hours: 1,
        },
    )
    .await
    .unwrap();

    let original_expires = ws.expires_at;
    let extended = db::workspace::extend_ttl(&pool, &ws.id.to_string(), 12)
        .await
        .unwrap();
    assert!(extended.expires_at > original_expires);
}

#[tokio::test]
async fn test_unique_branch_conflict() {
    let pool = common::setup_pool().await;
    let user = common::create_test_user(&pool).await;
    let repo = common::create_test_repo(&pool, &user.id.to_string()).await;

    db::workspace::create(
        &pool,
        db::workspace::CreateWorkspaceParams {
            repo_id: &repo.id.to_string(),
            creator_id: &user.id.to_string(),
            name: "dup",
            branch: "ws/dup/branch",
            base_branch: "main",
            base_commit: "abc",
            ttl_hours: 24,
        },
    )
    .await
    .unwrap();

    let result = db::workspace::create(
        &pool,
        db::workspace::CreateWorkspaceParams {
            repo_id: &repo.id.to_string(),
            creator_id: &user.id.to_string(),
            name: "dup2",
            branch: "ws/dup/branch",
            base_branch: "main",
            base_commit: "abc",
            ttl_hours: 24,
        },
    )
    .await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.to_string().contains("already exists"));
}

#[tokio::test]
async fn test_list_expired() {
    let pool = common::setup_pool().await;
    let user = common::create_test_user(&pool).await;
    let repo = common::create_test_repo(&pool, &user.id.to_string()).await;

    // Create workspace with 0-hour TTL (already expired via negative offset)
    let id = uuid::Uuid::new_v4().to_string();
    let past = (chrono::Utc::now() - chrono::Duration::hours(1)).to_rfc3339();
    let now = chrono::Utc::now().to_rfc3339();

    sqlx::query(
        "INSERT INTO workspaces (id, repo_id, creator_id, name, branch, base_branch, base_commit, status, ttl_hours, expires_at, created_at, updated_at)
         VALUES (?, ?, ?, 'expired-ws', 'ws/exp/test', 'main', 'abc', 'active', 1, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&repo.id.to_string())
    .bind(&user.id.to_string())
    .bind(&past)
    .bind(&now)
    .bind(&now)
    .execute(&pool)
    .await
    .unwrap();

    let expired = db::workspace::list_expired(&pool).await.unwrap();
    assert_eq!(expired.len(), 1);
    assert_eq!(expired[0].name, "expired-ws");
}
