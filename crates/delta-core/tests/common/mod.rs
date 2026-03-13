//! Shared test helpers for delta-core integration tests.

use delta_core::db;
use delta_core::models::repo::Visibility;

pub async fn setup_pool() -> sqlx::SqlitePool {
    let pool = sqlx::SqlitePool::connect("sqlite::memory:")
        .await
        .expect("failed to connect to in-memory db");

    for migration in [
        include_str!("../../migrations/001_initial.sql"),
        include_str!("../../migrations/002_git_protocol.sql"),
        include_str!("../../migrations/003_pull_requests.sql"),
        include_str!("../../migrations/004_cicd.sql"),
        include_str!("../../migrations/005_registry.sql"),
        include_str!("../../migrations/006_collaborators.sql"),
        include_str!("../../migrations/007_forks_and_templates.sql"),
        include_str!("../../migrations/008_lfs.sql"),
        include_str!("../../migrations/009_cascade_fixes.sql"),
        include_str!("../../migrations/010_search.sql"),
        include_str!("../../migrations/011_federation.sql"),
        include_str!("../../migrations/012_encryption.sql"),
        include_str!("../../migrations/013_workspaces.sql"),
    ] {
        sqlx::query(migration)
            .execute(&pool)
            .await
            .expect("failed to run migration");
    }

    pool
}

pub async fn create_test_user(pool: &sqlx::SqlitePool) -> delta_core::models::user::User {
    db::user::create(pool, "testuser", "test@example.com", "hashedpw", false)
        .await
        .expect("failed to create user")
}

pub async fn create_second_user(pool: &sqlx::SqlitePool) -> delta_core::models::user::User {
    db::user::create(pool, "otheruser", "other@example.com", "hashedpw", false)
        .await
        .expect("failed to create user")
}

pub async fn create_test_repo(
    pool: &sqlx::SqlitePool,
    owner_id: &str,
) -> delta_core::models::repo::Repository {
    db::repo::create(pool, owner_id, "testrepo", Some("desc"), Visibility::Public)
        .await
        .expect("failed to create repo")
}
