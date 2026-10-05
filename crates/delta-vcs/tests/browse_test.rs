//! Tests for repository browsing against real git repositories.

use std::path::{Path, PathBuf};
use std::process::Command;

fn git_in(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A bare repo whose HEAD contains the given files.
fn repo_with_files(tmp: &Path, files: &[(&str, &str)]) -> PathBuf {
    let work = tmp.join("work");
    for (path, content) in files {
        let path = work.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }
    git_in(&work, &["init", "-q", "-b", "main"]);
    git_in(&work, &["add", "."]);
    git_in(&work, &["commit", "-q", "-m", "init"]);
    let bare = tmp.join("repo.git");
    git_in(
        tmp,
        &[
            "clone",
            "-q",
            "--bare",
            work.to_str().unwrap(),
            bare.to_str().unwrap(),
        ],
    );
    bare
}

fn paths(entries: &[delta_vcs::browse::TreeEntry]) -> Vec<&str> {
    entries.iter().map(|e| e.path.as_str()).collect()
}

#[tokio::test]
async fn test_list_tree_lists_subdirectory_children() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = repo_with_files(
        tmp.path(),
        &[
            ("README.md", "hi"),
            ("docs/docs/intro.md", "intro"),
            ("docs/index.md", "index"),
            ("src/main.rs", "fn main() {}"),
        ],
    );

    let root = delta_vcs::browse::list_tree(&repo, "HEAD", "")
        .await
        .unwrap();
    assert_eq!(paths(&root), ["docs", "src", "README.md"]);

    // Listing a directory yields its children, not the directory itself.
    let docs = delta_vcs::browse::list_tree(&repo, "HEAD", "docs")
        .await
        .unwrap();
    assert_eq!(paths(&docs), ["docs/docs", "docs/index.md"]);
    let nested = delta_vcs::browse::list_tree(&repo, "HEAD", "docs/docs/")
        .await
        .unwrap();
    assert_eq!(paths(&nested), ["docs/docs/intro.md"]);
    assert_eq!(nested[0].name, "intro.md");
}

#[tokio::test]
async fn test_list_tree_and_blobs_handle_non_ascii_names() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = repo_with_files(
        tmp.path(),
        &[("docs/résumé.md", "cv"), ("big.txt", "12345")],
    );

    let docs = delta_vcs::browse::list_tree(&repo, "HEAD", "docs")
        .await
        .unwrap();
    assert_eq!(paths(&docs), ["docs/résumé.md"]);

    let mut blobs = delta_vcs::browse::list_blobs(&repo, "HEAD").await.unwrap();
    blobs.sort_by(|a, b| a.path.cmp(&b.path));
    let listed: Vec<(&str, u64)> = blobs.iter().map(|b| (b.path.as_str(), b.size)).collect();
    assert_eq!(listed, [("big.txt", 5), ("docs/résumé.md", 2)]);
}
