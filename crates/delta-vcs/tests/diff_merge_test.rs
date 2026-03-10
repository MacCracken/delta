use std::process::Command;
use tempfile::TempDir;

/// Create a non-bare repo with a commit, then return a bare clone of it.
fn setup_repo_with_branches(tmp: &TempDir) -> std::path::PathBuf {
    let work_dir = tmp.path().join("work");
    std::fs::create_dir_all(&work_dir).unwrap();

    // Init a regular repo
    Command::new("git")
        .args(["init", "--initial-branch=main"])
        .current_dir(&work_dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["config", "user.name", "Test User"])
        .current_dir(&work_dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["config", "user.email", "test@example.com"])
        .current_dir(&work_dir)
        .output()
        .unwrap();

    // Create a file and commit on main
    std::fs::write(work_dir.join("README.md"), "# Hello").unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(&work_dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "initial commit"])
        .current_dir(&work_dir)
        .output()
        .unwrap();

    // Create a feature branch with changes
    Command::new("git")
        .args(["checkout", "-b", "feature"])
        .current_dir(&work_dir)
        .output()
        .unwrap();
    std::fs::write(work_dir.join("feature.txt"), "new feature").unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(&work_dir)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-m", "add feature"])
        .current_dir(&work_dir)
        .output()
        .unwrap();

    // Go back to main
    Command::new("git")
        .args(["checkout", "main"])
        .current_dir(&work_dir)
        .output()
        .unwrap();

    // Clone as bare
    let bare_dir = tmp.path().join("bare.git");
    Command::new("git")
        .args([
            "clone",
            "--bare",
            work_dir.to_str().unwrap(),
            bare_dir.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    bare_dir
}

#[tokio::test]
async fn test_diff_refs() {
    let tmp = TempDir::new().unwrap();
    let bare = setup_repo_with_branches(&tmp);

    let diff = delta_vcs::diff::diff_refs(&bare, "main", "feature")
        .await
        .unwrap();
    assert!(diff.contains("feature.txt"));
}

#[tokio::test]
async fn test_diff_refs_invalid_ref() {
    let tmp = TempDir::new().unwrap();
    let bare = setup_repo_with_branches(&tmp);

    let result = delta_vcs::diff::diff_refs(&bare, "main", "../etc/passwd").await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_diff_stat() {
    let tmp = TempDir::new().unwrap();
    let bare = setup_repo_with_branches(&tmp);

    let stat = delta_vcs::diff::diff_stat(&bare, "main", "feature")
        .await
        .unwrap();
    assert!(stat.files_changed >= 1);
    assert!(stat.additions > 0);
    // At least one file should be feature.txt
    assert!(stat.files.iter().any(|f| f.path == "feature.txt"));
}

#[tokio::test]
async fn test_list_commits() {
    let tmp = TempDir::new().unwrap();
    let bare = setup_repo_with_branches(&tmp);

    let commits = delta_vcs::diff::list_commits(&bare, "main", "feature")
        .await
        .unwrap();
    assert_eq!(commits.len(), 1);
    assert_eq!(commits[0].message, "add feature");
    assert_eq!(commits[0].author_name, "Test User");
}

#[tokio::test]
async fn test_check_mergeable() {
    let tmp = TempDir::new().unwrap();
    let bare = setup_repo_with_branches(&tmp);

    let mergeable = delta_vcs::diff::check_mergeable(&bare, "main", "feature")
        .await
        .unwrap();
    assert!(mergeable);
}

#[tokio::test]
async fn test_execute_merge() {
    let tmp = TempDir::new().unwrap();
    let bare = setup_repo_with_branches(&tmp);

    let sha = delta_vcs::merge::execute_merge(
        &bare,
        "main",
        "feature",
        delta_vcs::merge::MergeMode::Merge,
        "Merge feature into main",
        "Test User",
        "test@example.com",
    )
    .await
    .unwrap();

    assert!(!sha.is_empty());
    assert!(sha.len() >= 40);
}

#[tokio::test]
async fn test_execute_merge_squash() {
    let tmp = TempDir::new().unwrap();
    let bare = setup_repo_with_branches(&tmp);

    let sha = delta_vcs::merge::execute_merge(
        &bare,
        "main",
        "feature",
        delta_vcs::merge::MergeMode::Squash,
        "Squash merge feature",
        "Test User",
        "test@example.com",
    )
    .await
    .unwrap();

    assert!(!sha.is_empty());
}

#[tokio::test]
async fn test_execute_merge_invalid_ref() {
    let tmp = TempDir::new().unwrap();
    let bare = setup_repo_with_branches(&tmp);

    let result = delta_vcs::merge::execute_merge(
        &bare,
        "main",
        "../etc/passwd",
        delta_vcs::merge::MergeMode::Merge,
        "msg",
        "Test",
        "test@example.com",
    )
    .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_execute_merge_invalid_author() {
    let tmp = TempDir::new().unwrap();
    let bare = setup_repo_with_branches(&tmp);

    let result = delta_vcs::merge::execute_merge(
        &bare,
        "main",
        "feature",
        delta_vcs::merge::MergeMode::Merge,
        "msg",
        "",
        "test@example.com",
    )
    .await;
    assert!(result.is_err());

    let result = delta_vcs::merge::execute_merge(
        &bare,
        "main",
        "feature",
        delta_vcs::merge::MergeMode::Merge,
        "msg",
        "-malicious",
        "test@example.com",
    )
    .await;
    assert!(result.is_err());
}
