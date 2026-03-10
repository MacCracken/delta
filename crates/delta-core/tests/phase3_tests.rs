use delta_core::db;
use delta_core::models::pull_request::*;
use delta_core::models::repo::Visibility;

async fn setup_pool() -> sqlx::SqlitePool {
    let pool = sqlx::SqlitePool::connect("sqlite::memory:")
        .await
        .expect("failed to connect");

    for migration in [
        include_str!("../migrations/001_initial.sql"),
        include_str!("../migrations/002_git_protocol.sql"),
        include_str!("../migrations/003_pull_requests.sql"),
    ] {
        sqlx::query(migration)
            .execute(&pool)
            .await
            .expect("migration failed");
    }

    pool
}

struct TestFixture {
    pool: sqlx::SqlitePool,
    user_id: String,
    reviewer_id: String,
    repo_id: String,
}

async fn setup_fixture() -> TestFixture {
    let pool = setup_pool().await;
    let user = db::user::create(&pool, "alice", "alice@example.com", "pw", false)
        .await
        .unwrap();
    let reviewer = db::user::create(&pool, "bob", "bob@example.com", "pw", false)
        .await
        .unwrap();
    let repo = db::repo::create(
        &pool,
        &user.id.to_string(),
        "myrepo",
        None,
        Visibility::Public,
    )
    .await
    .unwrap();
    TestFixture {
        pool,
        user_id: user.id.to_string(),
        reviewer_id: reviewer.id.to_string(),
        repo_id: repo.id.to_string(),
    }
}

async fn create_test_pr(f: &TestFixture) -> PullRequest {
    db::pull_request::create(
        &f.pool,
        db::pull_request::CreatePrParams {
            repo_id: &f.repo_id,
            author_id: &f.user_id,
            title: "Add feature",
            body: Some("This adds a new feature"),
            head_branch: "feature",
            base_branch: "main",
            head_sha: Some("abc123"),
            is_draft: false,
        },
    )
    .await
    .unwrap()
}

// --- Pull Request CRUD ---

#[tokio::test]
async fn test_create_pull_request() {
    let f = setup_fixture().await;
    let pr = create_test_pr(&f).await;

    assert_eq!(pr.number, 1);
    assert_eq!(pr.title, "Add feature");
    assert_eq!(pr.state, PrState::Open);
    assert_eq!(pr.head_branch, "feature");
    assert_eq!(pr.base_branch, "main");
    assert!(!pr.is_draft);
}

#[tokio::test]
async fn test_pr_number_increments() {
    let f = setup_fixture().await;
    let pr1 = create_test_pr(&f).await;
    let pr2 = db::pull_request::create(
        &f.pool,
        db::pull_request::CreatePrParams {
            repo_id: &f.repo_id,
            author_id: &f.user_id,
            title: "Second PR",
            body: None,
            head_branch: "feature2",
            base_branch: "main",
            head_sha: None,
            is_draft: false,
        },
    )
    .await
    .unwrap();

    assert_eq!(pr1.number, 1);
    assert_eq!(pr2.number, 2);
}

#[tokio::test]
async fn test_get_pr_by_number() {
    let f = setup_fixture().await;
    let pr = create_test_pr(&f).await;

    let fetched = db::pull_request::get_by_number(&f.pool, &f.repo_id, pr.number)
        .await
        .unwrap();
    assert_eq!(fetched.id, pr.id);
    assert_eq!(fetched.title, "Add feature");
}

#[tokio::test]
async fn test_list_prs_with_filter() {
    let f = setup_fixture().await;
    let pr = create_test_pr(&f).await;

    // All
    let all = db::pull_request::list_for_repo(&f.pool, &f.repo_id, None)
        .await
        .unwrap();
    assert_eq!(all.len(), 1);

    // Open
    let open = db::pull_request::list_for_repo(&f.pool, &f.repo_id, Some("open"))
        .await
        .unwrap();
    assert_eq!(open.len(), 1);

    // Close it
    db::pull_request::close(&f.pool, &pr.id.to_string())
        .await
        .unwrap();

    let open = db::pull_request::list_for_repo(&f.pool, &f.repo_id, Some("open"))
        .await
        .unwrap();
    assert_eq!(open.len(), 0);

    let closed = db::pull_request::list_for_repo(&f.pool, &f.repo_id, Some("closed"))
        .await
        .unwrap();
    assert_eq!(closed.len(), 1);
}

#[tokio::test]
async fn test_update_pr() {
    let f = setup_fixture().await;
    let pr = create_test_pr(&f).await;

    let updated = db::pull_request::update_title_body(
        &f.pool,
        &pr.id.to_string(),
        Some("Updated title"),
        Some("Updated body"),
    )
    .await
    .unwrap();

    assert_eq!(updated.title, "Updated title");
    assert_eq!(updated.body.as_deref(), Some("Updated body"));
}

#[tokio::test]
async fn test_close_and_reopen_pr() {
    let f = setup_fixture().await;
    let pr = create_test_pr(&f).await;

    let closed = db::pull_request::close(&f.pool, &pr.id.to_string())
        .await
        .unwrap();
    assert_eq!(closed.state, PrState::Closed);
    assert!(closed.closed_at.is_some());

    let reopened = db::pull_request::reopen(&f.pool, &pr.id.to_string())
        .await
        .unwrap();
    assert_eq!(reopened.state, PrState::Open);
    assert!(reopened.closed_at.is_none());
}

#[tokio::test]
async fn test_mark_merged() {
    let f = setup_fixture().await;
    let pr = create_test_pr(&f).await;

    let merged = db::pull_request::mark_merged(&f.pool, &pr.id.to_string(), &f.user_id, "squash")
        .await
        .unwrap();

    assert_eq!(merged.state, PrState::Merged);
    assert!(merged.merged_at.is_some());
    assert_eq!(merged.merge_strategy, Some(MergeStrategy::Squash));
}

// --- Comments ---

#[tokio::test]
async fn test_pr_comments() {
    let f = setup_fixture().await;
    let pr = create_test_pr(&f).await;
    let pr_id = pr.id.to_string();

    // General comment
    let c1 =
        db::pull_request::add_comment(&f.pool, &pr_id, &f.user_id, "Looks good!", None, None, None)
            .await
            .unwrap();
    assert_eq!(c1.body, "Looks good!");
    assert!(c1.file_path.is_none());

    // Inline comment
    let c2 = db::pull_request::add_comment(
        &f.pool,
        &pr_id,
        &f.reviewer_id,
        "Nit: rename this",
        Some("src/lib.rs"),
        Some(42),
        Some("right"),
    )
    .await
    .unwrap();
    assert_eq!(c2.file_path.as_deref(), Some("src/lib.rs"));
    assert_eq!(c2.line, Some(42));

    // List
    let comments = db::pull_request::list_comments(&f.pool, &pr_id)
        .await
        .unwrap();
    assert_eq!(comments.len(), 2);

    // Update
    let updated = db::pull_request::update_comment(&f.pool, &c1.id.to_string(), "LGTM!")
        .await
        .unwrap();
    assert_eq!(updated.body, "LGTM!");

    // Delete
    db::pull_request::delete_comment(&f.pool, &c2.id.to_string())
        .await
        .unwrap();
    let comments = db::pull_request::list_comments(&f.pool, &pr_id)
        .await
        .unwrap();
    assert_eq!(comments.len(), 1);
}

// --- Reviews ---

#[tokio::test]
async fn test_pr_reviews() {
    let f = setup_fixture().await;
    let pr = create_test_pr(&f).await;
    let pr_id = pr.id.to_string();

    // Submit review
    let review = db::pull_request::submit_review(
        &f.pool,
        &pr_id,
        &f.reviewer_id,
        ReviewState::ChangesRequested,
        Some("Please fix the tests"),
    )
    .await
    .unwrap();
    assert_eq!(review.state, ReviewState::ChangesRequested);

    // List
    let reviews = db::pull_request::list_reviews(&f.pool, &pr_id)
        .await
        .unwrap();
    assert_eq!(reviews.len(), 1);

    // Approve
    db::pull_request::submit_review(&f.pool, &pr_id, &f.reviewer_id, ReviewState::Approved, None)
        .await
        .unwrap();

    let reviews = db::pull_request::list_reviews(&f.pool, &pr_id)
        .await
        .unwrap();
    assert_eq!(reviews.len(), 2);
}

#[tokio::test]
async fn test_count_approvals() {
    let f = setup_fixture().await;
    let pr = create_test_pr(&f).await;
    let pr_id = pr.id.to_string();

    // No approvals yet
    let count = db::pull_request::count_approvals(&f.pool, &pr_id)
        .await
        .unwrap();
    assert_eq!(count, 0);

    // Add approval
    db::pull_request::submit_review(&f.pool, &pr_id, &f.reviewer_id, ReviewState::Approved, None)
        .await
        .unwrap();

    let count = db::pull_request::count_approvals(&f.pool, &pr_id)
        .await
        .unwrap();
    assert_eq!(count, 1);
}

// --- Status Checks ---

#[tokio::test]
async fn test_status_checks() {
    let f = setup_fixture().await;

    // Create a pending check
    let check = db::status_check::upsert(
        &f.pool,
        &f.repo_id,
        "abc123",
        "ci/tests",
        CheckState::Pending,
        Some("Running tests..."),
        Some("https://ci.example.com/1"),
    )
    .await
    .unwrap();
    assert_eq!(check.state, CheckState::Pending);
    assert_eq!(check.context, "ci/tests");

    // Not all passed
    let passed = db::status_check::all_passed(&f.pool, &f.repo_id, "abc123")
        .await
        .unwrap();
    assert!(!passed);

    // Update to success
    db::status_check::upsert(
        &f.pool,
        &f.repo_id,
        "abc123",
        "ci/tests",
        CheckState::Success,
        Some("All tests passed"),
        None,
    )
    .await
    .unwrap();

    let passed = db::status_check::all_passed(&f.pool, &f.repo_id, "abc123")
        .await
        .unwrap();
    assert!(passed);

    // Add a failing check
    db::status_check::upsert(
        &f.pool,
        &f.repo_id,
        "abc123",
        "ci/lint",
        CheckState::Failure,
        Some("Lint failed"),
        None,
    )
    .await
    .unwrap();

    let passed = db::status_check::all_passed(&f.pool, &f.repo_id, "abc123")
        .await
        .unwrap();
    assert!(!passed);

    // List checks
    let checks = db::status_check::get_for_commit(&f.pool, &f.repo_id, "abc123")
        .await
        .unwrap();
    assert_eq!(checks.len(), 2);
}

#[tokio::test]
async fn test_no_checks_means_passed() {
    let f = setup_fixture().await;

    let passed = db::status_check::all_passed(&f.pool, &f.repo_id, "nonexistent")
        .await
        .unwrap();
    assert!(passed);
}
