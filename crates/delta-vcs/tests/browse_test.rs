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

/// Commit in a work tree whose bare clone is `repo`, then push.
fn commit_as(work: &Path, author: &str, files: &[(&str, &str)], message: &str) {
    for (path, content) in files {
        std::fs::write(work.join(path), content).unwrap();
    }
    git_in(work, &["add", "."]);
    git_in(
        work,
        &[
            "-c",
            &format!("user.name={author}"),
            "commit",
            "-q",
            "--author",
            &format!("{author} <{author}@example.com>"),
            "-m",
            message,
        ],
    );
}

fn push(work: &Path, repo: &Path) {
    git_in(work, &["push", "-q", repo.to_str().unwrap(), "main"]);
}

#[tokio::test]
async fn test_read_blob_enforces_size_limit() {
    let tmp = tempfile::tempdir().unwrap();
    let big = "x".repeat(4096);
    let repo = repo_with_files(tmp.path(), &[("big.txt", &big), ("dir/a.txt", "a")]);

    assert_eq!(
        delta_vcs::browse::blob_size(&repo, "HEAD", "big.txt")
            .await
            .unwrap(),
        4096
    );
    let content = delta_vcs::browse::read_blob(&repo, "HEAD", "big.txt", 4096)
        .await
        .unwrap();
    assert_eq!(content.len(), 4096);
    let err = delta_vcs::browse::read_blob(&repo, "HEAD", "big.txt", 4095)
        .await
        .unwrap_err();
    assert!(matches!(err, delta_core::DeltaError::TooLarge(_)), "{err}");

    // Missing paths and directories are not files.
    assert!(matches!(
        delta_vcs::browse::read_blob(&repo, "HEAD", "nope.txt", 4096).await,
        Err(delta_core::DeltaError::NotFound(_))
    ));
    assert!(
        delta_vcs::browse::blob_size(&repo, "HEAD", "dir")
            .await
            .is_err()
    );
    assert!(
        delta_vcs::browse::read_blob(&repo, "HEAD", "dir", 4096)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn test_blame_attributes_every_line_to_its_commit() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = repo_with_files(tmp.path(), &[("f.txt", "a1\na2\na3\n")]);
    let work = tmp.path().join("work");
    // Carol changes the middle line, so the first commit's lines come before
    // and after hers. git prints a commit's author only with its first line.
    commit_as(&work, "Carol", &[("f.txt", "a1\nCAROL\na3\n")], "carol");
    commit_as(&work, "Bob", &[("f.txt", "a1\nCAROL\na3\nbob\n")], "bob");
    push(&work, &repo);

    let lines = delta_vcs::browse::blame(&repo, "HEAD", "f.txt", 1024)
        .await
        .unwrap();
    let authors: Vec<&str> = lines.iter().map(|l| l.author.as_str()).collect();
    assert_eq!(authors, ["Test", "Carol", "Test", "Bob"]);
    assert_eq!(lines[0].date, lines[2].date);
    assert!(lines.iter().all(|l| !l.date.is_empty()));
    assert_eq!(
        lines.iter().map(|l| l.line_number).collect::<Vec<_>>(),
        [1, 2, 3, 4]
    );

    assert!(matches!(
        delta_vcs::browse::blame(&repo, "HEAD", "f.txt", 4).await,
        Err(delta_core::DeltaError::TooLarge(_))
    ));
}

#[tokio::test]
async fn test_log_cannot_be_forged_by_commit_messages() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = repo_with_files(tmp.path(), &[("f.txt", "1")]);
    let work = tmp.path().join("work");
    let forged = format!(
        "innocent\n\n---END---\n{}\nAdmin\nadmin@example.com\n2020-01-01T00:00:00+00:00\nforged entry\n",
        "a".repeat(40)
    );
    commit_as(&work, "Mallory", &[("f.txt", "2")], &forged);
    push(&work, &repo);

    let entries = delta_vcs::browse::log(&repo, "HEAD", None, 10)
        .await
        .unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].author_name, "Mallory");
    assert_eq!(entries[0].message, "innocent");
    assert!(entries[0].body.contains("forged entry"));
    assert!(entries.iter().all(|e| e.author_name != "Admin"));
    let commits = delta_vcs::diff::list_commits(&repo, &entries[1].sha, &entries[0].sha)
        .await
        .unwrap();
    assert_eq!(commits.len(), 1);
    assert_eq!(commits[0].author_name, "Mallory");
}

#[tokio::test]
async fn test_show_commit_stats_and_large_diffs() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = repo_with_files(tmp.path(), &[("a b.txt", "one\n"), ("old.txt", "keep\n")]);
    let work = tmp.path().join("work");

    // The root commit is compared with the empty tree.
    let root = delta_vcs::browse::show_commit(&repo, "HEAD").await.unwrap();
    assert!(root.parents.is_empty());
    assert_eq!(root.stats.len(), 2);
    assert!(
        root.stats
            .iter()
            .any(|s| s.path == "a b.txt" && s.additions == 1)
    );
    assert!(root.diff.contains("+one"));
    assert!(!root.diff_truncated);

    git_in(&work, &["mv", "old.txt", "new.txt"]);
    commit_as(&work, "Test", &[("a b.txt", "one\ntwo\n")], "edit");
    push(&work, &repo);
    let commit = delta_vcs::browse::show_commit(&repo, "HEAD").await.unwrap();
    assert_eq!(commit.parents.len(), 1);
    let mut paths: Vec<&str> = commit.stats.iter().map(|s| s.path.as_str()).collect();
    paths.sort();
    assert_eq!(paths, ["a b.txt", "new.txt"]);

    // A diff over the cap is left out instead of buffered.
    let line = "y".repeat(1023) + "\n";
    let huge = line.repeat(delta_vcs::browse::MAX_DIFF_BYTES / 1024 + 16);
    commit_as(&work, "Test", &[("huge.txt", &huge)], "huge");
    push(&work, &repo);
    let commit = delta_vcs::browse::show_commit(&repo, "HEAD").await.unwrap();
    assert!(commit.diff_truncated);
    assert!(commit.diff.is_empty());
    assert_eq!(commit.stats.len(), 1);
    let before = commit.parents[0].clone();
    assert!(matches!(
        delta_vcs::diff::diff_refs(&repo, &before, "HEAD").await,
        Err(delta_core::DeltaError::TooLarge(_))
    ));
}
