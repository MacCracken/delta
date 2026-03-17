-- Self-hosted CI runners
CREATE TABLE IF NOT EXISTS runners (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL UNIQUE,
    token_hash TEXT NOT NULL,
    labels TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL DEFAULT 'offline',
    last_heartbeat_at TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_runners_status ON runners(status);

-- Pending job queue for remote runners.
-- When a job targets a self-hosted runner, it is inserted here so runners
-- can poll for work. Completed/failed jobs are cleaned up after reporting.
CREATE TABLE IF NOT EXISTS runner_job_queue (
    id TEXT PRIMARY KEY NOT NULL,
    job_run_id TEXT NOT NULL REFERENCES job_runs(id) ON DELETE CASCADE,
    pipeline_id TEXT NOT NULL REFERENCES pipeline_runs(id) ON DELETE CASCADE,
    repo_id TEXT NOT NULL,
    labels TEXT NOT NULL DEFAULT '',
    payload TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    claimed_by TEXT REFERENCES runners(id) ON DELETE SET NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_runner_job_queue_status ON runner_job_queue(status);
CREATE INDEX IF NOT EXISTS idx_runner_job_queue_labels ON runner_job_queue(labels);
