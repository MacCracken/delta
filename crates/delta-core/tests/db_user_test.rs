//! Tests for user, token, password, and SSH key DB operations.

mod common;

use delta_core::db;
use delta_core::models::repo::Visibility;

#[tokio::test]
async fn test_create_and_get_user() {
    let pool = common::setup_pool().await;
    let user = common::create_test_user(&pool).await;

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
    let pool = common::setup_pool().await;
    common::create_test_user(&pool).await;

    let user = db::user::get_by_username(&pool, "testuser").await.unwrap();
    assert_eq!(user.email, "test@example.com");
}

#[tokio::test]
async fn test_duplicate_user_fails() {
    let pool = common::setup_pool().await;
    common::create_test_user(&pool).await;

    let result = db::user::create(&pool, "testuser", "other@example.com", "pw", false).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_password_hash_lookup() {
    let pool = common::setup_pool().await;
    common::create_test_user(&pool).await;

    let (id, hash) = db::user::get_password_hash(&pool, "testuser")
        .await
        .unwrap();
    assert!(!id.is_empty());
    assert_eq!(hash, "hashedpw");
}

#[tokio::test]
async fn test_api_token_lifecycle() {
    let pool = common::setup_pool().await;
    let user = common::create_test_user(&pool).await;
    let uid = user.id.to_string();

    let token_hash = "somehash123";
    let token_id = db::user::create_token(&pool, &uid, "test-token", token_hash, "*", None)
        .await
        .unwrap();

    let found = db::user::get_by_token_hash(&pool, token_hash)
        .await
        .unwrap();
    assert!(found.is_some());
    assert_eq!(found.unwrap().username, "testuser");

    let not_found = db::user::get_by_token_hash(&pool, "badhash").await.unwrap();
    assert!(not_found.is_none());

    db::user::delete_token(&pool, &token_id, &uid)
        .await
        .unwrap();

    let gone = db::user::get_by_token_hash(&pool, token_hash)
        .await
        .unwrap();
    assert!(gone.is_none());
}

#[tokio::test]
async fn test_list_tokens() {
    let pool = common::setup_pool().await;
    let user = common::create_test_user(&pool).await;
    let uid = user.id.to_string();

    let tokens = db::user::list_tokens(&pool, &uid).await.unwrap();
    assert!(tokens.is_empty());

    db::user::create_token(&pool, &uid, "token-a", "hash_a", "*", None)
        .await
        .unwrap();
    db::user::create_token(&pool, &uid, "token-b", "hash_b", "read", None)
        .await
        .unwrap();

    let tokens = db::user::list_tokens(&pool, &uid).await.unwrap();
    assert_eq!(tokens.len(), 2);
    assert_eq!(tokens[0].name, "token-b");
    assert_eq!(tokens[1].name, "token-a");
    assert_eq!(tokens[1].scopes, "*");
    assert_eq!(tokens[0].scopes, "read");
}

#[tokio::test]
async fn test_list_tokens_excludes_other_users() {
    let pool = common::setup_pool().await;
    let user1 = common::create_test_user(&pool).await;
    let user2 = db::user::create(&pool, "other", "other@example.com", "pw", false)
        .await
        .unwrap();
    let uid1 = user1.id.to_string();
    let uid2 = user2.id.to_string();

    db::user::create_token(&pool, &uid1, "u1-token", "h1", "*", None)
        .await
        .unwrap();
    db::user::create_token(&pool, &uid2, "u2-token", "h2", "*", None)
        .await
        .unwrap();

    let tokens1 = db::user::list_tokens(&pool, &uid1).await.unwrap();
    assert_eq!(tokens1.len(), 1);
    assert_eq!(tokens1[0].name, "u1-token");

    let tokens2 = db::user::list_tokens(&pool, &uid2).await.unwrap();
    assert_eq!(tokens2.len(), 1);
    assert_eq!(tokens2[0].name, "u2-token");
}

// --- SSH key tests ---

#[tokio::test]
async fn test_ssh_key_crud() {
    let pool = common::setup_pool().await;
    let user = common::create_test_user(&pool).await;
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

    let fetched = db::ssh_key::get_by_id(&pool, &key.id.to_string())
        .await
        .unwrap();
    assert_eq!(fetched.name, "laptop");

    let keys = db::ssh_key::list_by_user(&pool, &uid).await.unwrap();
    assert_eq!(keys.len(), 1);

    let result = db::ssh_key::get_user_by_fingerprint(&pool, "SHA256:testfingerprint123")
        .await
        .unwrap();
    assert!(result.is_some());
    let (found_uid, found_username) = result.unwrap();
    assert_eq!(found_uid, uid);
    assert_eq!(found_username, "testuser");

    db::ssh_key::delete(&pool, &key.id.to_string(), &uid)
        .await
        .unwrap();
    let keys = db::ssh_key::list_by_user(&pool, &uid).await.unwrap();
    assert!(keys.is_empty());
}

#[tokio::test]
async fn test_ssh_key_delete_wrong_user() {
    let pool = common::setup_pool().await;
    let user = common::create_test_user(&pool).await;
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
    let pool = common::setup_pool().await;

    let result = db::ssh_key::get_user_by_fingerprint(&pool, "SHA256:nonexistent")
        .await
        .unwrap();
    assert!(result.is_none());
}
