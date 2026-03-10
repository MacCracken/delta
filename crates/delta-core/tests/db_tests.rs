use delta_core::db;
use delta_core::models::repo::Visibility;

async fn setup_pool() -> sqlx::SqlitePool {
    let pool = sqlx::SqlitePool::connect("sqlite::memory:")
        .await
        .expect("failed to connect to in-memory db");

    sqlx::query(include_str!("../migrations/001_initial.sql"))
        .execute(&pool)
        .await
        .expect("failed to run migrations");

    pool
}

async fn create_test_user(pool: &sqlx::SqlitePool) -> delta_core::models::user::User {
    db::user::create(pool, "testuser", "test@example.com", "hashedpw", false)
        .await
        .expect("failed to create user")
}

#[tokio::test]
async fn test_create_and_get_user() {
    let pool = setup_pool().await;
    let user = create_test_user(&pool).await;

    assert_eq!(user.username, "testuser");
    assert_eq!(user.email, "test@example.com");
    assert!(!user.is_agent);

    let fetched = db::user::get_by_id(&pool, &user.id.to_string())
        .await
        .unwrap();
    assert_eq!(fetched.username, "testuser");
}

#[tokio::test]
async fn test_get_user_by_username() {
    let pool = setup_pool().await;
    create_test_user(&pool).await;

    let user = db::user::get_by_username(&pool, "testuser").await.unwrap();
    assert_eq!(user.email, "test@example.com");
}

#[tokio::test]
async fn test_duplicate_user_fails() {
    let pool = setup_pool().await;
    create_test_user(&pool).await;

    let result = db::user::create(&pool, "testuser", "other@example.com", "pw", false).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_create_and_get_repo() {
    let pool = setup_pool().await;
    let user = create_test_user(&pool).await;
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
    let pool = setup_pool().await;
    let user = create_test_user(&pool).await;
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
    let pool = setup_pool().await;
    let user = create_test_user(&pool).await;
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
    let pool = setup_pool().await;
    let user = create_test_user(&pool).await;
    let uid = user.id.to_string();

    db::repo::create(&pool, &uid, "public-repo", None, Visibility::Public)
        .await
        .unwrap();
    db::repo::create(&pool, &uid, "private-repo", None, Visibility::Private)
        .await
        .unwrap();

    // Anonymous sees only public
    let repos = db::repo::list_visible(&pool, None).await.unwrap();
    assert_eq!(repos.len(), 1);
    assert_eq!(repos[0].name, "public-repo");

    // Owner sees both
    let repos = db::repo::list_visible(&pool, Some(&uid)).await.unwrap();
    assert_eq!(repos.len(), 2);
}

#[tokio::test]
async fn test_update_repo() {
    let pool = setup_pool().await;
    let user = create_test_user(&pool).await;
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
    let pool = setup_pool().await;
    let user = create_test_user(&pool).await;
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
    let pool = setup_pool().await;
    let user = create_test_user(&pool).await;
    let uid = user.id.to_string();

    db::repo::create(&pool, &uid, "myrepo", None, Visibility::Private)
        .await
        .unwrap();

    let result = db::repo::create(&pool, &uid, "myrepo", None, Visibility::Private).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_api_token_lifecycle() {
    let pool = setup_pool().await;
    let user = create_test_user(&pool).await;
    let uid = user.id.to_string();

    let token_hash = "somehash123";
    let token_id = db::user::create_token(&pool, &uid, "test-token", token_hash, "*", None)
        .await
        .unwrap();

    // Look up by token hash
    let found = db::user::get_by_token_hash(&pool, token_hash)
        .await
        .unwrap();
    assert!(found.is_some());
    assert_eq!(found.unwrap().username, "testuser");

    // Invalid hash returns None
    let not_found = db::user::get_by_token_hash(&pool, "badhash").await.unwrap();
    assert!(not_found.is_none());

    // Delete token
    db::user::delete_token(&pool, &token_id, &uid)
        .await
        .unwrap();

    let gone = db::user::get_by_token_hash(&pool, token_hash)
        .await
        .unwrap();
    assert!(gone.is_none());
}

#[tokio::test]
async fn test_password_hash_lookup() {
    let pool = setup_pool().await;
    create_test_user(&pool).await;

    let (id, hash) = db::user::get_password_hash(&pool, "testuser")
        .await
        .unwrap();
    assert!(!id.is_empty());
    assert_eq!(hash, "hashedpw");
}
