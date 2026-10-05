//! Tests for audit log and audit export DB operations.

mod common;

use delta_core::db;

// --- Audit log tests ---

#[tokio::test]
async fn test_audit_log_and_list() {
    let pool = common::setup_pool().await;
    let user = common::create_test_user(&pool).await;
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
    let pool = common::setup_pool().await;
    let user = common::create_test_user(&pool).await;
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
    let pool = common::setup_pool().await;

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
    let pool = common::setup_pool().await;

    db::audit::log(&pool, None, "boot", "system", None, None, None)
        .await
        .unwrap();

    let entries = db::audit::list(&pool, None, None, 50, 0).await.unwrap();
    assert_eq!(entries.len(), 1);
}

#[tokio::test]
async fn test_audit_log_pagination() {
    let pool = common::setup_pool().await;

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

#[tokio::test]
async fn test_audit_log_and_list_detailed() {
    let pool = common::setup_pool().await;
    let user = common::create_test_user(&pool).await;
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
    let pool = common::setup_pool().await;
    let user = common::create_test_user(&pool).await;
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
    let pool = common::setup_pool().await;

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
    let pool = common::setup_pool().await;
    let user = common::create_test_user(&pool).await;
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

// --- Audit export tests ---

#[tokio::test]
async fn test_audit_list_for_export() {
    let pool = common::setup_pool().await;
    let user = common::create_test_user(&pool).await;
    let uid = user.id.to_string();

    for action in ["create", "update", "delete"] {
        db::audit::log(&pool, Some(&uid), action, "repo", Some("repo1"), None, None)
            .await
            .unwrap();
    }

    let entries = db::audit::list_for_export(&pool, &Default::default(), 100, 0)
        .await
        .unwrap();
    assert_eq!(entries.len(), 3);

    let entries = db::audit::list_for_export(
        &pool,
        &db::audit::ExportFilter {
            resource_type: Some("repo"),
            ..Default::default()
        },
        100,
        0,
    )
    .await
    .unwrap();
    assert_eq!(entries.len(), 3);

    let entries = db::audit::list_for_export(
        &pool,
        &db::audit::ExportFilter {
            resource_type: Some("nonexistent"),
            ..Default::default()
        },
        100,
        0,
    )
    .await
    .unwrap();
    assert!(entries.is_empty());
}

#[tokio::test]
async fn test_audit_count_for_export() {
    let pool = common::setup_pool().await;
    let user = common::create_test_user(&pool).await;
    let uid = user.id.to_string();

    for _ in 0..5 {
        db::audit::log(&pool, Some(&uid), "push", "repo", Some("r1"), None, None)
            .await
            .unwrap();
    }

    let count = db::audit::count_for_export(&pool, &Default::default())
        .await
        .unwrap();
    assert_eq!(count, 5);
}

#[tokio::test]
async fn test_audit_export_pagination() {
    let pool = common::setup_pool().await;
    let user = common::create_test_user(&pool).await;
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

    let page1 = db::audit::list_for_export(&pool, &Default::default(), 3, 0)
        .await
        .unwrap();
    assert_eq!(page1.len(), 3);

    let page2 = db::audit::list_for_export(&pool, &Default::default(), 3, 3)
        .await
        .unwrap();
    assert_eq!(page2.len(), 3);

    assert_ne!(page1[0].id, page2[0].id);
}

#[tokio::test]
async fn test_audit_export_filters_by_user_and_time() {
    let pool = common::setup_pool().await;
    let alice = common::create_test_user(&pool).await.id.to_string();
    let bob = common::create_second_user(&pool).await.id.to_string();
    for user in [&alice, &bob] {
        db::audit::log(&pool, Some(user), "login", "user", None, None, None)
            .await
            .unwrap();
    }

    let only_alice = db::audit::ExportFilter {
        user_id: Some(&alice),
        ..Default::default()
    };
    let entries = db::audit::list_for_export(&pool, &only_alice, 100, 0)
        .await
        .unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].user_id.as_deref(), Some(alice.as_str()));

    // Time bounds compare as times: an hour ago (with a different
    // offset and separator) is before now; an hour ahead is after.
    let hour_ago = (chrono::Utc::now() - chrono::Duration::hours(1))
        .with_timezone(&chrono::FixedOffset::east_opt(5 * 3600).unwrap())
        .to_rfc3339();
    let hour_ahead = (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339();
    let window = |since, until| db::audit::ExportFilter {
        since,
        until,
        ..only_alice
    };
    for (since, until, expected) in [
        (Some(hour_ago.as_str()), None, 1),
        (Some(hour_ahead.as_str()), None, 0),
        (None, Some(hour_ago.as_str()), 0),
        (None, Some(hour_ahead.as_str()), 1),
    ] {
        let count = db::audit::count_for_export(&pool, &window(since, until))
            .await
            .unwrap();
        assert_eq!(count, expected, "since {since:?} until {until:?}");
    }
}
