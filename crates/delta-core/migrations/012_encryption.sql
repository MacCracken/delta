-- Phase 8: End-to-end encrypted repository support.

CREATE TABLE IF NOT EXISTS repo_encryption_keys (
    id TEXT PRIMARY KEY NOT NULL,
    repo_id TEXT NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    encrypted_key TEXT NOT NULL,
    algorithm TEXT NOT NULL DEFAULT 'xchacha20-poly1305',
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(repo_id, user_id)
);

ALTER TABLE repositories ADD COLUMN encrypted BOOLEAN NOT NULL DEFAULT FALSE;

CREATE INDEX IF NOT EXISTS idx_repo_encryption_keys_repo ON repo_encryption_keys(repo_id);
