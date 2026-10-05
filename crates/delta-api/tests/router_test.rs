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

/// Send a JSON request through the router; returns status and raw body.
async fn send(
    app: &Router,
    method: &str,
    uri: &str,
    token: Option<&str>,
    body: serde_json::Value,
) -> (StatusCode, String) {
    let mut req = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json");
    if let Some(token) = token {
        req = req.header("authorization", format!("Bearer {token}"));
    }
    let res = app
        .clone()
        .oneshot(req.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

/// Register a user through the API; returns their token.
async fn register(app: &Router, username: &str) -> String {
    let (status, body) = send(
        app,
        "POST",
        "/api/v1/auth/register",
        None,
        serde_json::json!({
            "username": username,
            "email": format!("{username}@example.com"),
            "password": "correct horse battery staple",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    json["token"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn mcp_list_repos_by_owner_hides_private_repos() {
    let tmp = tempfile::tempdir().unwrap();
    let app = test_app(&tmp).await;
    let token = register(&app, "alice").await;
    for (name, visibility) in [("open-repo", "public"), ("secret-repo", "private")] {
        let (status, body) = send(
            &app,
            "POST",
            "/api/v1/repos",
            Some(&token),
            serde_json::json!({ "name": name, "visibility": visibility }),
        )
        .await;
        assert!(status.is_success(), "{body}");
    }

    let (status, body) = send(
        &app,
        "POST",
        "/v1/mcp/tools/call",
        None,
        serde_json::json!({ "name": "delta_list_repos", "arguments": { "owner": "alice" } }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("open-repo"), "{body}");
    assert!(!body.contains("secret-repo"), "private repo leaked: {body}");
}
