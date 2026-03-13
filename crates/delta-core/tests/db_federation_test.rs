//! Tests for federation instance DB operations.

mod common;

use delta_core::db;

#[tokio::test]
async fn test_federation_add_instance() {
    let pool = common::setup_pool().await;
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
    let pool = common::setup_pool().await;
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
    let pool = common::setup_pool().await;
    let inst = db::federation::add_instance(&pool, "https://c.example.com", None, None, false)
        .await
        .unwrap();

    let fetched = db::federation::get_instance(&pool, &inst.id).await.unwrap();
    assert_eq!(fetched.url, "https://c.example.com");
}

#[tokio::test]
async fn test_federation_get_instance_by_url() {
    let pool = common::setup_pool().await;
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
    let pool = common::setup_pool().await;
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
    let pool = common::setup_pool().await;
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
    let pool = common::setup_pool().await;
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
    let pool = common::setup_pool().await;
    db::federation::add_instance(&pool, "https://h.example.com", None, None, false)
        .await
        .unwrap();

    let result =
        db::federation::add_instance(&pool, "https://h.example.com", None, None, false).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_federation_delete_nonexistent() {
    let pool = common::setup_pool().await;
    let result = db::federation::delete_instance(&pool, "nonexistent-id").await;
    assert!(result.is_err());
}
