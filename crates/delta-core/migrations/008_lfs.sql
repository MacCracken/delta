-- Git LFS (Large File Storage) support
CREATE TABLE IF NOT EXISTS lfs_objects (
    id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL REFERENCES repositories(id),
    oid TEXT NOT NULL,           -- SHA-256 hash (LFS spec)
    size INTEGER NOT NULL,
    created_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(repo_id, oid)
);

CREATE INDEX IF NOT EXISTS idx_lfs_objects_repo ON lfs_objects(repo_id);
CREATE INDEX IF NOT EXISTS idx_lfs_objects_oid ON lfs_objects(oid);
