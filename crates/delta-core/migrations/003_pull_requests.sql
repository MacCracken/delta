-- Pull requests
CREATE TABLE IF NOT EXISTS pull_requests (
    id TEXT PRIMARY KEY NOT NULL,
    number INTEGER NOT NULL,
    repo_id TEXT NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    author_id TEXT NOT NULL REFERENCES users(id),
    title TEXT NOT NULL,
    body TEXT,
    state TEXT NOT NULL DEFAULT 'open',
    head_branch TEXT NOT NULL,
    base_branch TEXT NOT NULL,
    head_sha TEXT,
    is_draft BOOLEAN NOT NULL DEFAULT FALSE,
    merged_by TEXT REFERENCES users(id),
    merge_strategy TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    merged_at TEXT,
    closed_at TEXT,
    UNIQUE(repo_id, number)
);

-- PR comments (general + inline/file-level)
CREATE TABLE IF NOT EXISTS pr_comments (
    id TEXT PRIMARY KEY NOT NULL,
    pr_id TEXT NOT NULL REFERENCES pull_requests(id) ON DELETE CASCADE,
    author_id TEXT NOT NULL REFERENCES users(id),
    body TEXT NOT NULL,
    file_path TEXT,
    line INTEGER,
    side TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- PR reviews
CREATE TABLE IF NOT EXISTS pr_reviews (
    id TEXT PRIMARY KEY NOT NULL,
    pr_id TEXT NOT NULL REFERENCES pull_requests(id) ON DELETE CASCADE,
    reviewer_id TEXT NOT NULL REFERENCES users(id),
    state TEXT NOT NULL,
    body TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Status checks (commit statuses)
CREATE TABLE IF NOT EXISTS status_checks (
    id TEXT PRIMARY KEY NOT NULL,
    repo_id TEXT NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    commit_sha TEXT NOT NULL,
    context TEXT NOT NULL,
    state TEXT NOT NULL DEFAULT 'pending',
    description TEXT,
    target_url TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(repo_id, commit_sha, context)
);

-- PR number sequence per repo
CREATE TABLE IF NOT EXISTS pr_counters (
    repo_id TEXT PRIMARY KEY NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    next_number INTEGER NOT NULL DEFAULT 1
);

CREATE INDEX IF NOT EXISTS idx_pull_requests_repo ON pull_requests(repo_id);
CREATE INDEX IF NOT EXISTS idx_pull_requests_state ON pull_requests(repo_id, state);
CREATE INDEX IF NOT EXISTS idx_pr_comments_pr ON pr_comments(pr_id);
CREATE INDEX IF NOT EXISTS idx_pr_reviews_pr ON pr_reviews(pr_id);
CREATE INDEX IF NOT EXISTS idx_status_checks_commit ON status_checks(repo_id, commit_sha);
