-- CI/CD pipeline runs
CREATE TABLE IF NOT EXISTS pipeline_runs (
    id TEXT PRIMARY KEY NOT NULL,
    repo_id TEXT NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    workflow_name TEXT NOT NULL,
    trigger_type TEXT NOT NULL,
    trigger_ref TEXT,
    commit_sha TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'queued',
    started_at TEXT,
    finished_at TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Individual job runs within a pipeline
CREATE TABLE IF NOT EXISTS job_runs (
    id TEXT PRIMARY KEY NOT NULL,
    pipeline_id TEXT NOT NULL REFERENCES pipeline_runs(id) ON DELETE CASCADE,
    job_name TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'queued',
    runner TEXT,
    started_at TEXT,
    finished_at TEXT,
    exit_code INTEGER,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Step logs within a job
CREATE TABLE IF NOT EXISTS step_logs (
    id TEXT PRIMARY KEY NOT NULL,
    job_id TEXT NOT NULL REFERENCES job_runs(id) ON DELETE CASCADE,
    step_name TEXT NOT NULL,
    step_index INTEGER NOT NULL,
    output TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL DEFAULT 'pending',
    started_at TEXT,
    finished_at TEXT
);

-- Repository secrets (encrypted)
CREATE TABLE IF NOT EXISTS repo_secrets (
    id TEXT PRIMARY KEY NOT NULL,
    repo_id TEXT NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    encrypted_value TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(repo_id, name)
);

-- Artifacts and releases
CREATE TABLE IF NOT EXISTS artifacts (
    id TEXT PRIMARY KEY NOT NULL,
    repo_id TEXT NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    pipeline_id TEXT REFERENCES pipeline_runs(id) ON DELETE SET NULL,
    name TEXT NOT NULL,
    version TEXT,
    artifact_type TEXT NOT NULL DEFAULT 'generic',
    content_hash TEXT NOT NULL,
    size_bytes INTEGER NOT NULL,
    metadata TEXT,
    download_count INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Releases (tied to tags)
CREATE TABLE IF NOT EXISTS releases (
    id TEXT PRIMARY KEY NOT NULL,
    repo_id TEXT NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    tag_name TEXT NOT NULL,
    name TEXT NOT NULL,
    body TEXT,
    is_draft BOOLEAN NOT NULL DEFAULT FALSE,
    is_prerelease BOOLEAN NOT NULL DEFAULT FALSE,
    author_id TEXT NOT NULL REFERENCES users(id),
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(repo_id, tag_name)
);

-- Release assets
CREATE TABLE IF NOT EXISTS release_assets (
    id TEXT PRIMARY KEY NOT NULL,
    release_id TEXT NOT NULL REFERENCES releases(id) ON DELETE CASCADE,
    artifact_id TEXT NOT NULL REFERENCES artifacts(id) ON DELETE CASCADE,
    label TEXT
);

-- Audit log
CREATE TABLE IF NOT EXISTS audit_log (
    id TEXT PRIMARY KEY NOT NULL,
    user_id TEXT REFERENCES users(id),
    action TEXT NOT NULL,
    resource_type TEXT NOT NULL,
    resource_id TEXT,
    details TEXT,
    ip_address TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Federation instances
CREATE TABLE IF NOT EXISTS federation_instances (
    id TEXT PRIMARY KEY NOT NULL,
    url TEXT NOT NULL UNIQUE,
    name TEXT,
    public_key TEXT,
    trusted BOOLEAN NOT NULL DEFAULT FALSE,
    last_seen_at TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_pipeline_runs_repo ON pipeline_runs(repo_id);
CREATE INDEX IF NOT EXISTS idx_pipeline_runs_status ON pipeline_runs(repo_id, status);
CREATE INDEX IF NOT EXISTS idx_job_runs_pipeline ON job_runs(pipeline_id);
CREATE INDEX IF NOT EXISTS idx_step_logs_job ON step_logs(job_id);
CREATE INDEX IF NOT EXISTS idx_artifacts_repo ON artifacts(repo_id);
CREATE INDEX IF NOT EXISTS idx_releases_repo ON releases(repo_id);
CREATE INDEX IF NOT EXISTS idx_audit_log_user ON audit_log(user_id);
CREATE INDEX IF NOT EXISTS idx_audit_log_created ON audit_log(created_at);
CREATE INDEX IF NOT EXISTS idx_release_assets_release ON release_assets(release_id);
CREATE INDEX IF NOT EXISTS idx_audit_log_resource ON audit_log(resource_type, resource_id);
