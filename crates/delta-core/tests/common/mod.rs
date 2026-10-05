//! Shared test helpers for delta-core integration tests.

#![allow(dead_code)]

use delta_core::db;
use delta_core::models::repo::Visibility;

pub async fn setup_pool() -> sqlx::SqlitePool {
    let pool = sqlx::SqlitePool::connect("sqlite::memory:")
        .await
        .expect("failed to connect to in-memory db");

    db::run_migrations(&pool)
        .await
        .expect("failed to run migrations");

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
