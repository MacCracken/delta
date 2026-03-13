//! Tests for repository CRUD, branch protection, collaborators, and mirrors.

mod common;

use delta_core::db;
use delta_core::models::branch_protection::BranchProtection;
use delta_core::models::collaborator::CollaboratorRole;
use delta_core::models::repo::Visibility;
use uuid::Uuid;

// --- Repository CRUD ---

#[tokio::test]
async fn test_create_and_get_repo() {
    let pool = common::setup_pool().await;
    let user = common::create_test_user(&pool).await;
    let uid = user.id.to_string();

    let repo = db::repo::create(
        &pool,
        &uid,
        "myrepo",
        Some("A test repo"),
        Visibility::Public,
    )
    .await
    .unwrap();

    assert_eq!(repo.name, "myrepo");
    assert_eq!(repo.description.as_deref(), Some("A test repo"));
    assert_eq!(repo.visibility, Visibility::Public);

    let fetched = db::repo::get_by_id(&pool, &repo.id.to_string())
        .await
        .unwrap();
    assert_eq!(fetched.name, "myrepo");
}

#[tokio::test]
async fn test_get_repo_by_owner_and_name() {
    let pool = common::setup_pool().await;
    let user = common::create_test_user(&pool).await;
    let uid = user.id.to_string();

    db::repo::create(&pool, &uid, "myrepo", None, Visibility::Private)
        .await
        .unwrap();

    let repo = db::repo::get_by_owner_and_name(&pool, &uid, "myrepo")
        .await
        .unwrap();
    assert_eq!(repo.name, "myrepo");
}

#[tokio::test]
async fn test_list_repos_by_owner() {
    let pool = common::setup_pool().await;
    let user = common::create_test_user(&pool).await;
    let uid = user.id.to_string();

    db::repo::create(&pool, &uid, "repo1", None, Visibility::Public)
        .await
        .unwrap();
    db::repo::create(&pool, &uid, "repo2", None, Visibility::Private)
        .await
        .unwrap();

    let repos = db::repo::list_by_owner(&pool, &uid).await.unwrap();
    assert_eq!(repos.len(), 2);
}

#[tokio::test]
async fn test_list_visible_repos() {
    let pool = common::setup_pool().await;
    let user = common::create_test_user(&pool).await;
    let uid = user.id.to_string();

    db::repo::create(&pool, &uid, "public-repo", None, Visibility::Public)
        .await
        .unwrap();
    db::repo::create(&pool, &uid, "private-repo", None, Visibility::Private)
        .await
        .unwrap();

    let repos = db::repo::list_visible(&pool, None).await.unwrap();
    assert_eq!(repos.len(), 1);
    assert_eq!(repos[0].name, "public-repo");

    let repos = db::repo::list_visible(&pool, Some(&uid)).await.unwrap();
    assert_eq!(repos.len(), 2);
}

#[tokio::test]
async fn test_update_repo() {
    let pool = common::setup_pool().await;
    let user = common::create_test_user(&pool).await;
    let uid = user.id.to_string();

    let repo = db::repo::create(&pool, &uid, "myrepo", None, Visibility::Private)
        .await
        .unwrap();
    let rid = repo.id.to_string();

    let updated = db::repo::update(
        &pool,
        &rid,
        Some("Updated description"),
        Some(Visibility::Public),
        None,
    )
    .await
    .unwrap();

    assert_eq!(updated.description.as_deref(), Some("Updated description"));
    assert_eq!(updated.visibility, Visibility::Public);
}

#[tokio::test]
async fn test_delete_repo() {
    let pool = common::setup_pool().await;
    let user = common::create_test_user(&pool).await;
    let uid = user.id.to_string();

    let repo = db::repo::create(&pool, &uid, "myrepo", None, Visibility::Private)
        .await
        .unwrap();
    let rid = repo.id.to_string();

    db::repo::delete(&pool, &rid).await.unwrap();

    let result = db::repo::get_by_id(&pool, &rid).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_duplicate_repo_name_fails() {
    let pool = common::setup_pool().await;
    let user = common::create_test_user(&pool).await;
    let uid = user.id.to_string();

    db::repo::create(&pool, &uid, "myrepo", None, Visibility::Private)
        .await
        .unwrap();

    let result = db::repo::create(&pool, &uid, "myrepo", None, Visibility::Private).await;
    assert!(result.is_err());
}

// --- Branch protection ---

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
    let pool = common::setup_pool().await;
    let user = common::create_test_user(&pool).await;
    let repo = common::create_test_repo(&pool, &user.id.to_string()).await;
    let repo_id = repo.id.to_string();

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

    let protections = db::branch_protection::list_for_repo(&pool, &repo_id)
        .await
        .unwrap();
    assert_eq!(protections.len(), 1);

    let found = db::branch_protection::find_matching(&pool, &repo_id, "main")
        .await
        .unwrap();
    assert!(found.is_some());
    assert_eq!(found.unwrap().pattern, "main");

    let not_found = db::branch_protection::find_matching(&pool, &repo_id, "develop")
        .await
        .unwrap();
    assert!(not_found.is_none());

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
    let pool = common::setup_pool().await;
    let user = common::create_test_user(&pool).await;
    let repo = common::create_test_repo(&pool, &user.id.to_string()).await;
    let repo_id = repo.id.to_string();

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

// --- Collaborator tests ---

#[tokio::test]
async fn test_collaborator_set_and_get() {
    let pool = common::setup_pool().await;
    let user = common::create_test_user(&pool).await;
    let other = common::create_second_user(&pool).await;
    let repo = common::create_test_repo(&pool, &user.id.to_string()).await;

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
    let pool = common::setup_pool().await;
    let user = common::create_test_user(&pool).await;
    let other = common::create_second_user(&pool).await;
    let repo = common::create_test_repo(&pool, &user.id.to_string()).await;

    let role = db::collaborator::get_role(&pool, &repo.id.to_string(), &other.id.to_string())
        .await
        .expect("get_role failed");
    assert!(role.is_none());

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
    let pool = common::setup_pool().await;
    let user = common::create_test_user(&pool).await;
    let other = common::create_second_user(&pool).await;
    let repo = common::create_test_repo(&pool, &user.id.to_string()).await;

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
    let pool = common::setup_pool().await;
    let user = common::create_test_user(&pool).await;
    let other = common::create_second_user(&pool).await;
    let repo = common::create_test_repo(&pool, &user.id.to_string()).await;

    db::collaborator::set(
        &pool,
        &repo.id.to_string(),
        &other.id.to_string(),
        CollaboratorRole::Read,
    )
    .await
    .unwrap();

    let updated = db::collaborator::set(
        &pool,
        &repo.id.to_string(),
        &other.id.to_string(),
        CollaboratorRole::Write,
    )
    .await
    .unwrap();
    assert_eq!(updated.role, CollaboratorRole::Write);

    let list = db::collaborator::list_for_repo(&pool, &repo.id.to_string())
        .await
        .unwrap();
    assert_eq!(list.len(), 1);
}

#[tokio::test]
async fn test_collaborator_remove() {
    let pool = common::setup_pool().await;
    let user = common::create_test_user(&pool).await;
    let other = common::create_second_user(&pool).await;
    let repo = common::create_test_repo(&pool, &user.id.to_string()).await;

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
    let pool = common::setup_pool().await;
    let user = common::create_test_user(&pool).await;
    let repo = common::create_test_repo(&pool, &user.id.to_string()).await;

    let result = db::collaborator::remove(&pool, &repo.id.to_string(), "nonexistent-id").await;
    assert!(result.is_err());
}

// --- Mirror repo tests ---

#[tokio::test]
async fn test_create_mirror_repo() {
    let pool = common::setup_pool().await;
    let user = common::create_test_user(&pool).await;
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

// --- Webhook tests ---

#[tokio::test]
async fn test_webhook_crud() {
    let pool = common::setup_pool().await;
    let user = common::create_test_user(&pool).await;
    let repo = common::create_test_repo(&pool, &user.id.to_string()).await;
    let repo_id = repo.id.to_string();

    let id = db::webhook::create(
        &pool,
        &repo_id,
        "https://example.com/webhook",
        Some("mysecret"),
        r#"["push","tag_create"]"#,
    )
    .await
    .unwrap();

    let webhooks = db::webhook::list_for_repo(&pool, &repo_id).await.unwrap();
    assert_eq!(webhooks.len(), 1);
    assert_eq!(webhooks[0].url, "https://example.com/webhook");

    let push_hooks = db::webhook::get_for_event(&pool, &repo_id, "push")
        .await
        .unwrap();
    assert_eq!(push_hooks.len(), 1);

    let pr_hooks = db::webhook::get_for_event(&pool, &repo_id, "pull_request")
        .await
        .unwrap();
    assert_eq!(pr_hooks.len(), 0);

    db::webhook::delete(&pool, &id, &repo_id).await.unwrap();

    let webhooks = db::webhook::list_for_repo(&pool, &repo_id).await.unwrap();
    assert_eq!(webhooks.len(), 0);
}

#[tokio::test]
async fn test_webhook_delivery_recording() {
    let pool = common::setup_pool().await;
    let user = common::create_test_user(&pool).await;
    let repo = common::create_test_repo(&pool, &user.id.to_string()).await;
    let repo_id = repo.id.to_string();

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

    let count: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM webhook_deliveries WHERE webhook_id = ?")
            .bind(&webhook_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(count.0, 1);
}
