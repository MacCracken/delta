//! Integration tests for browsing repository content (web UI, raw files and
//! MCP tools).

mod common;

use common::{create_user_and_repo, git_ok, start_server};
use reqwest::StatusCode;
use serde_json::json;

#[tokio::test]
async fn large_files_are_offered_raw_instead_of_rendered() {
    let server = start_server().await;
    create_user_and_repo(&server, "dora", "assets", "public").await;
    let bare = server.state.repo_host.repo_path("dora", "assets").unwrap();
    let work = tempfile::tempdir().unwrap();
    let dir = work.path();
    git_ok(dir, &["init", "-q", "-b", "main"]).await;
    let big = common::incompressible(2 * 1024 * 1024);
    std::fs::write(dir.join("big.bin"), &big).unwrap();
    std::fs::write(dir.join("small.txt"), "hello world\n").unwrap();
    git_ok(dir, &["add", "."]).await;
    git_ok(dir, &["commit", "-q", "-m", "init"]).await;
    git_ok(dir, &["push", "-q", bare.to_str().unwrap(), "main"]).await;

    let client = reqwest::Client::new();
    let get = |path: &str| {
        client
            .get(format!("{}/dora/assets/-/{path}", server.base))
            .send()
    };

    // Small files are rendered; large ones point to the raw view.
    let page = get("blob/main/small.txt")
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(page.contains("hello world"));
    let res = get("blob/main/big.bin").await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert!(res.text().await.unwrap().contains("too large to display"));
    let res = get("blame/main/big.bin").await.unwrap();
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);

    // Raw files are streamed whole and never interpreted as a page.
    let res = get("raw/main/big.bin").await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.headers()["x-content-type-options"], "nosniff");
    assert_eq!(res.bytes().await.unwrap().as_ref(), big.as_slice());
    assert_eq!(
        get("raw/main/missing.txt").await.unwrap().status(),
        StatusCode::NOT_FOUND
    );

    // Tools get text files up to the limit.
    let read_file = |path: &str| {
        client
            .post(format!("{}/v1/mcp/tools/call", server.base))
            .json(&json!({
                "name": "delta_read_file",
                "arguments": { "owner": "dora", "name": "assets", "path": path },
            }))
            .send()
    };
    let res = read_file("small.txt").await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert!(res.text().await.unwrap().contains("hello world"));
    let res = read_file("big.bin").await.unwrap();
    assert_eq!(res.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

#[tokio::test]
async fn branches_with_slashes_and_trailing_slashes_resolve() {
    let server = start_server().await;
    create_user_and_repo(&server, "flo", "site", "public").await;
    let bare = server.state.repo_host.repo_path("flo", "site").unwrap();
    let work = tempfile::tempdir().unwrap();
    let dir = work.path();
    git_ok(dir, &["init", "-q", "-b", "main"]).await;
    std::fs::create_dir_all(dir.join("docs")).unwrap();
    std::fs::write(dir.join("docs/guide.md"), "on main\n").unwrap();
    git_ok(dir, &["add", "."]).await;
    git_ok(dir, &["commit", "-q", "-m", "init"]).await;
    git_ok(dir, &["checkout", "-q", "-b", "feature/new-docs"]).await;
    std::fs::write(dir.join("docs/guide.md"), "on the feature branch\n").unwrap();
    git_ok(dir, &["commit", "-q", "-am", "feature"]).await;
    git_ok(
        dir,
        &[
            "push",
            "-q",
            bare.to_str().unwrap(),
            "main",
            "feature/new-docs",
        ],
    )
    .await;

    let client = reqwest::Client::new();
    let page = |path: &str| {
        let request = client.get(format!("{}/flo/site{path}", server.base));
        async move {
            let res = request.send().await.unwrap();
            (res.status(), res.text().await.unwrap())
        }
    };

    let (status, body) = page("/-/blob/feature/new-docs/docs/guide.md").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.contains("on the feature branch"));
    let (status, body) = page("/-/raw/feature/new-docs/docs/guide.md").await;
    assert_eq!(
        (status, body.as_str()),
        (StatusCode::OK, "on the feature branch\n")
    );
    let (status, body) = page("/-/tree/feature/new-docs/docs").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.contains("guide.md"));
    let (status, _) = page("/-/tree/feature/new-docs").await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = page("/-/commits/feature/new-docs").await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = page("/-/blob/main/docs/guide.md").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("on main"));

    // Trailing slashes, as in hand-edited or generated links.
    for path in ["/", "/-/tree/main/", "/-/tree/main/docs/"] {
        let (status, _) = page(path).await;
        assert_eq!(status, StatusCode::OK, "{path}");
    }
}
