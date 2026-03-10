use delta_vcs::RepoHost;
use tempfile::TempDir;

#[test]
fn test_init_bare_creates_valid_git_repo() {
    let tmp = TempDir::new().unwrap();
    let host = RepoHost::new(tmp.path());

    let path = host.init_bare("alice", "myproject").unwrap();
    assert!(path.exists());

    // Should be a valid bare git repo (has HEAD, refs/, objects/)
    assert!(path.join("HEAD").exists());
    assert!(path.join("refs").exists());
    assert!(path.join("objects").exists());
}

#[test]
fn test_init_bare_conflict() {
    let tmp = TempDir::new().unwrap();
    let host = RepoHost::new(tmp.path());

    host.init_bare("alice", "repo").unwrap();
    let result = host.init_bare("alice", "repo");
    assert!(result.is_err());
}

#[test]
fn test_list_and_delete() {
    let tmp = TempDir::new().unwrap();
    let host = RepoHost::new(tmp.path());

    host.init_bare("alice", "repo1").unwrap();
    host.init_bare("alice", "repo2").unwrap();

    let repos = host.list_repos("alice").unwrap();
    assert_eq!(repos.len(), 2);
    assert!(repos.contains(&"repo1".to_string()));
    assert!(repos.contains(&"repo2".to_string()));

    host.delete("alice", "repo1").unwrap();
    let repos = host.list_repos("alice").unwrap();
    assert_eq!(repos.len(), 1);
}

#[test]
fn test_refs_on_empty_repo() {
    let tmp = TempDir::new().unwrap();
    let host = RepoHost::new(tmp.path());
    let path = host.init_bare("alice", "empty").unwrap();

    // Empty repo should have no branches
    let branches = delta_vcs::refs::list_branches(&path).unwrap();
    assert!(branches.is_empty());

    let tags = delta_vcs::refs::list_tags(&path).unwrap();
    assert!(tags.is_empty());

    let head = delta_vcs::refs::head_commit(&path).unwrap();
    assert!(head.is_none());
}

#[test]
fn test_refs_after_commit() {
    let tmp = TempDir::new().unwrap();

    // Create a non-bare repo, make a commit, then check refs
    let repo_path = tmp.path().join("workrepo");
    std::fs::create_dir_all(&repo_path).unwrap();

    // Use git commands to init, add, commit
    let status = std::process::Command::new("git")
        .args(["init", "--initial-branch=main"])
        .current_dir(&repo_path)
        .output()
        .unwrap();
    assert!(status.status.success());

    std::process::Command::new("git")
        .args(["config", "user.email", "test@test.com"])
        .current_dir(&repo_path)
        .output()
        .unwrap();

    std::process::Command::new("git")
        .args(["config", "user.name", "Test"])
        .current_dir(&repo_path)
        .output()
        .unwrap();

    std::fs::write(repo_path.join("README.md"), "# Hello").unwrap();

    std::process::Command::new("git")
        .args(["add", "."])
        .current_dir(&repo_path)
        .output()
        .unwrap();

    std::process::Command::new("git")
        .args(["commit", "-m", "initial"])
        .current_dir(&repo_path)
        .output()
        .unwrap();

    // Create a tag
    std::process::Command::new("git")
        .args(["tag", "v0.1.0"])
        .current_dir(&repo_path)
        .output()
        .unwrap();

    // Now check refs
    let branches = delta_vcs::refs::list_branches(&repo_path).unwrap();
    assert_eq!(branches.len(), 1);
    assert_eq!(branches[0].name, "main");
    assert!(branches[0].is_default);
    assert!(!branches[0].commit_id.is_empty());

    let tags = delta_vcs::refs::list_tags(&repo_path).unwrap();
    assert_eq!(tags.len(), 1);
    assert_eq!(tags[0].name, "v0.1.0");

    let head = delta_vcs::refs::head_commit(&repo_path).unwrap();
    assert!(head.is_some());
    assert_eq!(head.unwrap(), branches[0].commit_id);
}

#[tokio::test]
async fn test_advertise_refs_on_bare_repo() {
    let tmp = TempDir::new().unwrap();
    let host = RepoHost::new(tmp.path());
    let path = host.init_bare("alice", "testrepo").unwrap();

    let result =
        delta_vcs::protocol::advertise_refs(&path, "git-upload-pack").await;
    // Should succeed even on empty repo
    assert!(result.is_ok());
    let body = result.unwrap();
    let body_str = String::from_utf8_lossy(&body);
    assert!(body_str.contains("# service=git-upload-pack"));
}

#[tokio::test]
async fn test_advertise_refs_invalid_service() {
    let tmp = TempDir::new().unwrap();
    let host = RepoHost::new(tmp.path());
    let path = host.init_bare("alice", "testrepo").unwrap();

    let result =
        delta_vcs::protocol::advertise_refs(&path, "git-evil-command").await;
    assert!(result.is_err());
}
