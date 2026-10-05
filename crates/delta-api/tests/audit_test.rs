//! Integration tests for the audit log export.

mod common;

use common::{register, start_server};
use reqwest::StatusCode;

#[tokio::test]
async fn audit_export_filters_by_time() {
    let server = start_server().await;
    let token = register(&server, "ada").await;
    let export = |query: &str| {
        reqwest::Client::new()
            .get(format!("{}/api/v1/audit/export?{query}", server.base))
            .bearer_auth(&token)
            .send()
    };
    let entries = |body: serde_json::Value| body["entries"].as_array().unwrap().len();

    // Registration was logged; a window that ends before it is empty.
    let body = export("since=2000-01-01")
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(entries(body), 1);
    let body = export("until=2000-01-01T00:00:00Z")
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(entries(body), 0);

    let res = export("since=yesterday").await.unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
    // A negative limit is not "unlimited".
    let body: serde_json::Value = export("limit=-1").await.unwrap().json().await.unwrap();
    assert_eq!(body["limit"], 1);
}
