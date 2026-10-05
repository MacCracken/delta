//! Integration tests for the OCI and ark registries.

mod common;

use base64::Engine;
use common::{create_user_and_repo, register, start_server};
use reqwest::StatusCode;
use sha2::{Digest, Sha256};

fn basic(user: &str, token: &str) -> String {
    let creds = base64::engine::general_purpose::STANDARD.encode(format!("{user}:{token}"));
    format!("Basic {creds}")
}

fn sha256_digest(data: &[u8]) -> String {
    format!("sha256:{}", hex::encode(Sha256::digest(data)))
}

#[tokio::test]
async fn oci_auth_handshake_push_and_pull() {
    let server = start_server().await;
    let token = create_user_and_repo(&server, "alice", "img", "public").await;
    create_user_and_repo(&server, "bob", "unused", "public").await;
    let client = reqwest::Client::new();
    let v2 = format!("{}/v2/", server.base);

    // The version check challenges anonymous clients so `docker login` sends
    // credentials, and accepts the Basic credentials it then sends.
    let res = client.get(&v2).send().await.unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    let challenge = res.headers()["www-authenticate"].to_str().unwrap();
    assert!(challenge.starts_with("Basic "), "{challenge}");
    let body: serde_json::Value = res.json().await.unwrap();
    assert_eq!(body["errors"][0]["code"], "UNAUTHORIZED");
    let res = client
        .get(&v2)
        .header("authorization", basic("alice", &token))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    // A token is only valid for its own user name.
    let res = client
        .get(&v2)
        .header("authorization", basic("bob", &token))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    // Monolithic blob upload larger than axum's default 2 MiB body limit.
    let layer: Vec<u8> = (0..3 * 1024 * 1024u32).map(|i| (i % 251) as u8).collect();
    let layer_digest = sha256_digest(&layer);
    let res = client
        .post(format!(
            "{}/v2/alice/img/blobs/uploads/?digest={layer_digest}",
            server.base
        ))
        .header("authorization", basic("alice", &token))
        .body(layer)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::CREATED);

    let manifest = serde_json::json!({
        "schemaVersion": 2,
        "mediaType": "application/vnd.oci.image.manifest.v1+json",
        "layers": [{ "digest": layer_digest }],
    })
    .to_string();
    let manifest_url = format!("{}/v2/alice/img/manifests/v1", server.base);
    let res = client
        .put(&manifest_url)
        .header("authorization", basic("alice", &token))
        .header("content-type", "application/vnd.oci.image.manifest.v1+json")
        .body(manifest.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::CREATED);

    // Public images can be pulled anonymously.
    let res = client.get(&manifest_url).send().await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.text().await.unwrap(), manifest);

    // Writes still require credentials.
    let res = client
        .put(&manifest_url)
        .body(manifest.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    // A manifest pushed by digest must match that digest.
    let wrong = format!(
        "{}/v2/alice/img/manifests/sha256:{}",
        server.base,
        "0".repeat(64)
    );
    let res = client
        .put(&wrong)
        .header("authorization", basic("alice", &token))
        .body(manifest)
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn oci_private_repo_requires_credentials() {
    let server = start_server().await;
    let token = create_user_and_repo(&server, "carol", "secret", "private").await;
    let client = reqwest::Client::new();
    let url = format!("{}/v2/carol/secret/tags/list", server.base);
    let res = client.get(&url).send().await.unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    let res = client
        .get(&url)
        .header("authorization", basic("carol", &token))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
}

#[tokio::test]
async fn ark_package_names_belong_to_their_publisher() {
    let server = start_server().await;
    let alice = create_user_and_repo(&server, "alice", "pkgs", "public").await;
    let mallory = create_user_and_repo(&server, "mallory", "junk", "public").await;
    let client = reqwest::Client::new();
    let publish = |token: String, version: &'static str| {
        let client = client.clone();
        let url = format!("{}/api/v1/registry/ark/libfoo/{version}", server.base);
        async move {
            let meta = serde_json::json!({
                "name": "libfoo",
                "version": version,
                "arch": "x86_64",
            });
            client
                .put(url)
                .bearer_auth(token)
                .header("x-ark-meta", meta.to_string())
                .body(b"package bytes".to_vec())
                .send()
                .await
                .unwrap()
                .status()
        }
    };

    assert_eq!(publish(alice.clone(), "1.0.0").await, StatusCode::CREATED);
    assert_eq!(publish(mallory, "1.0.1").await, StatusCode::FORBIDDEN);
    assert_eq!(publish(alice, "1.0.1").await, StatusCode::CREATED);

    let versions: Vec<serde_json::Value> = client
        .get(format!("{}/api/v1/registry/ark/libfoo", server.base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(versions.len(), 2);
}

#[tokio::test]
async fn ark_packages_follow_repository_visibility() {
    let server = start_server().await;
    let dave = create_user_and_repo(&server, "dave", "internal", "private").await;
    let outsider = register(&server, "erin").await;
    let client = reqwest::Client::new();
    let meta = serde_json::json!({ "name": "tool", "version": "1.0.0", "arch": "x86_64" });
    let status = client
        .put(format!("{}/api/v1/registry/ark/tool/1.0.0", server.base))
        .bearer_auth(&dave)
        .header("x-ark-meta", meta.to_string())
        .body(b"secret tool".to_vec())
        .send()
        .await
        .unwrap()
        .status();
    assert_eq!(status, StatusCode::CREATED);

    let url = format!("{}/api/v1/registry/ark/tool/1.0.0?arch=x86_64", server.base);
    // Published into a private repository: hidden from anonymous users and
    // non-members, available to the owner.
    assert_eq!(
        client.get(&url).send().await.unwrap().status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        client
            .get(&url)
            .bearer_auth(&outsider)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    let res = client.get(&url).bearer_auth(&dave).send().await.unwrap();
    let status = res.status();
    assert_eq!(status, StatusCode::OK, "{}", res.text().await.unwrap());
}

#[tokio::test]
async fn lfs_batch_returns_absolute_urls_and_uploads_are_idempotent() {
    let server = start_server().await;
    let token = create_user_and_repo(&server, "lena", "media", "public").await;
    let client = reqwest::Client::new();
    let data = b"large file contents".to_vec();
    let oid = hex::encode(Sha256::digest(&data));

    let batch: serde_json::Value = client
        .post(format!(
            "{}/lena/media.git/info/lfs/objects/batch",
            server.base
        ))
        .header("authorization", basic("lena", &token))
        .header("content-type", "application/vnd.git-lfs+json")
        .json(&serde_json::json!({
            "operation": "upload",
            "transfers": ["basic"],
            "objects": [{ "oid": oid, "size": data.len() }],
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let href = batch["objects"][0]["actions"]["upload"]["href"]
        .as_str()
        .unwrap()
        .to_string();
    // git-lfs rejects relative URLs ("missing protocol").
    assert!(href.starts_with("http://"), "{href}");

    for _ in 0..2 {
        let status = client
            .put(&href)
            .header("authorization", basic("lena", &token))
            .body(data.clone())
            .send()
            .await
            .unwrap()
            .status();
        assert_eq!(status, StatusCode::OK);
    }
}
