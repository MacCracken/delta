//! Tests for encryption key DB operations.

mod common;

use delta_core::db;

#[tokio::test]
async fn test_encryption_add_and_get_key() {
    let pool = common::setup_pool().await;
    let user = common::create_test_user(&pool).await;
    let repo = common::create_test_repo(&pool, &user.id.to_string()).await;

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
    let pool = common::setup_pool().await;
    let user1 = common::create_test_user(&pool).await;
    let user2 = db::user::create(&pool, "user2", "u2@test.com", "pw", false)
        .await
        .unwrap();
    let repo = common::create_test_repo(&pool, &user1.id.to_string()).await;

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
    let pool = common::setup_pool().await;
    let user = common::create_test_user(&pool).await;
    let repo = common::create_test_repo(&pool, &user.id.to_string()).await;

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
    let pool = common::setup_pool().await;
    let user = common::create_test_user(&pool).await;
    let repo = common::create_test_repo(&pool, &user.id.to_string()).await;

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
    let pool = common::setup_pool().await;
    let user = common::create_test_user(&pool).await;
    let repo = common::create_test_repo(&pool, &user.id.to_string()).await;

    db::encryption::set_repo_encrypted(&pool, &repo.id.to_string(), true)
        .await
        .unwrap();

    let encrypted: bool = sqlx::query_scalar("SELECT encrypted FROM repositories WHERE id = ?")
        .bind(&repo.id.to_string())
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(encrypted);
}
