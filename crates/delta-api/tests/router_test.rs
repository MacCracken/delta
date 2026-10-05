//! Integration tests that exercise the fully assembled HTTP router.

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use delta_api::{routes, state::AppState};
use delta_core::DeltaConfig;
use tower::ServiceExt;

/// Build the complete application router backed by a fresh on-disk database.
async fn test_app(tmp: &tempfile::TempDir) -> Router {
    let mut config = DeltaConfig::default();
    config.storage.repos_dir = tmp.path().join("repos");
    config.storage.artifacts_dir = tmp.path().join("artifacts");
    config.storage.db_url = format!(
        "sqlite://{}?mode=rwc",
        tmp.path().join("delta.db").display()
    );
    std::fs::create_dir_all(&config.storage.repos_dir).unwrap();
    std::fs::create_dir_all(&config.storage.artifacts_dir).unwrap();

    let pool = delta_core::db::init_pool(&config.storage.db_url)
        .await
        .unwrap();
    routes::router(AppState::new(config, pool))
}

#[tokio::test]
async fn router_builds_and_serves_health() {
    let tmp = tempfile::tempdir().unwrap();
    // Building the router panics on invalid route syntax, so this also guards
    // against route definitions that axum rejects at startup.
    let app = test_app(&tmp).await;

    let res = app
        .oneshot(Request::get("/health").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
}
