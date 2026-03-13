CREATE TABLE IF NOT EXISTS workspaces (
    id TEXT PRIMARY KEY NOT NULL,
    repo_id TEXT NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    creator_id TEXT NOT NULL REFERENCES users(id),
    name TEXT NOT NULL,
    branch TEXT NOT NULL,
    base_branch TEXT NOT NULL,
    base_commit TEXT NOT NULL,
    head_commit TEXT,
    status TEXT NOT NULL DEFAULT 'active',
    ttl_hours INTEGER NOT NULL DEFAULT 24,
    expires_at TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(repo_id, branch)
);
CREATE INDEX IF NOT EXISTS idx_workspaces_repo ON workspaces(repo_id);
CREATE INDEX IF NOT EXISTS idx_workspaces_creator ON workspaces(creator_id);
CREATE INDEX IF NOT EXISTS idx_workspaces_expires ON workspaces(expires_at);
