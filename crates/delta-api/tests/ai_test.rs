//! Integration tests for AI features against a stub provider.

mod common;

use common::{create_user_and_repo, start_server_with};
use reqwest::StatusCode;

#[tokio::test]
async fn provider_errors_are_not_passed_to_clients() {
    // A provider that fails with details clients must not see.
    let stub = axum::Router::new().fallback(|| async {
        (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "billing account acct_secret123 suspended",
        )
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, stub).await.unwrap() });

    let server = start_server_with(|config| {
        config.rate_limit.enabled = false;
        config.ai.enabled = true;
        config.ai.provider = delta_core::config::AiProvider::Hoosh;
        config.ai.endpoint = Some(endpoint);
    })
    .await;
    let token = create_user_and_repo(&server, "ida", "proj", "public").await;
    let res = reqwest::Client::new()
        .post(format!("{}/api/v1/repos/ida/proj/ai/query", server.base))
        .bearer_auth(&token)
        .json(&serde_json::json!({ "question": "what is this?" }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_GATEWAY);
    let body = res.text().await.unwrap();
    assert!(body.contains("AI provider returned an error"), "{body}");
    assert!(!body.contains("acct_secret123"), "{body}");
}
