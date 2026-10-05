//! Rate limiting must key on the real client, not on client-supplied headers.

mod common;

use common::{register, start_server_with};

#[tokio::test]
async fn login_attempts_are_limited_per_client() {
    let server = start_server_with(|config| {
        config.rate_limit.enabled = true;
        config.rate_limit.auth_requests_per_window = 3;
    })
    .await;
    register(&server, "alice").await;

    let client = reqwest::Client::new();
    let mut statuses = Vec::new();
    for i in 0..6 {
        let status = client
            .post(format!("{}/api/v1/auth/login", server.base))
            // Rotating forwarded-for values must not evade the limit.
            .header("x-forwarded-for", format!("203.0.113.{i}"))
            .json(&serde_json::json!({ "username": "alice", "password": "wrong password" }))
            .send()
            .await
            .unwrap()
            .status();
        statuses.push(status.as_u16());
    }
    // Registration used one attempt from this client already.
    assert!(statuses.contains(&429), "never limited: {statuses:?}");
    assert_eq!(statuses.last(), Some(&429));
}
