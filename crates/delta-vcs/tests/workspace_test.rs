//! Tests for agent workspace commits.

use delta_vcs::workspace::{FileWrite, commit_workspace_files, create_workspace_branch};
use std::path::Path;
use std::process::Command;

fn git_in(dir: &Path, args: &[&str]) -> String {
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
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

/// A bare repo whose `main` contains `link -> <target>` (a symlink).
fn repo_with_symlink(tmp: &Path, target: &Path) -> std::path::PathBuf {
    let work = tmp.join("work");
    std::fs::create_dir_all(&work).unwrap();
    git_in(&work, &["init", "-q", "-b", "main"]);
    std::fs::write(work.join("README.md"), "hi").unwrap();
    std::os::unix::fs::symlink(target, work.join("link")).unwrap();
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

fn write(path: &str, content: &str) -> FileWrite {
    FileWrite {
        path: path.into(),
        content: Some(content.as_bytes().to_vec()),
    }
}

#[tokio::test]
async fn test_workspace_write_refuses_symlinks() {
    let tmp = tempfile::tempdir().unwrap();
    let outside = tmp.path().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    let victim = outside.join("victim.txt");
    std::fs::write(&victim, "original").unwrap();

    // The symlink points at a directory outside the repository.
    let bare = repo_with_symlink(tmp.path(), &outside);
    create_workspace_branch(&bare, "ws/test", "main")
        .await
        .unwrap();

    for path in ["link/victim.txt", "link/new.txt", "link"] {
        let result = commit_workspace_files(
            &bare,
            "ws/test",
            &[write(path, "pwned")],
            "write",
            "Agent",
            "agent@example.com",
        )
        .await;
        assert!(result.is_err(), "write to {path} should be refused");
    }
    assert_eq!(std::fs::read_to_string(&victim).unwrap(), "original");
    assert!(!outside.join("new.txt").exists());

    // Ordinary writes still work, and the author is recorded.
    let sha = commit_workspace_files(
        &bare,
        "ws/test",
        &[write("src/new.txt", "ok")],
        "add file",
        "Agent",
        "agent@example.com",
    )
    .await
    .unwrap();
    assert_eq!(git_in(&bare, &["log", "-1", "--format=%an", &sha]), "Agent");
    assert!(
        !std::fs::read_to_string(bare.join("config"))
            .unwrap()
            .contains("Agent")
    );
}

#[tokio::test]
async fn test_workspace_write_refuses_git_dir_paths() {
    let tmp = tempfile::tempdir().unwrap();
    let bare = repo_with_symlink(tmp.path(), Path::new("/nonexistent"));
    create_workspace_branch(&bare, "ws/git", "main")
        .await
        .unwrap();
    for path in [".git", ".git/config", "sub/.GIT/hooks/x"] {
        let result = commit_workspace_files(
            &bare,
            "ws/git",
            &[write(path, "gitdir: /elsewhere")],
            "write",
            "Agent",
            "agent@example.com",
        )
        .await;
        assert!(result.is_err(), "write to {path} should be refused");
    }
}
