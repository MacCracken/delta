-- Phase 5: Registry completion — retention, download stats, signing, ark packages, OCI

-- Artifact retention policies (per-repo, overrides global config defaults)
CREATE TABLE IF NOT EXISTS artifact_retention_policies (
    id TEXT PRIMARY KEY NOT NULL,
    repo_id TEXT NOT NULL UNIQUE REFERENCES repositories(id) ON DELETE CASCADE,
    max_age_days INTEGER,
    max_count INTEGER,
    max_total_bytes INTEGER,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Download event tracking (per-download granularity)
CREATE TABLE IF NOT EXISTS download_events (
    id TEXT PRIMARY KEY NOT NULL,
    artifact_id TEXT NOT NULL REFERENCES artifacts(id) ON DELETE CASCADE,
    user_id TEXT REFERENCES users(id),
    user_agent TEXT,
    ip_address TEXT,
    downloaded_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_download_events_artifact ON download_events(artifact_id);
CREATE INDEX IF NOT EXISTS idx_download_events_date ON download_events(downloaded_at);

-- User signing keys (ed25519 public keys)
CREATE TABLE IF NOT EXISTS user_signing_keys (
    id TEXT PRIMARY KEY NOT NULL,
    user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    public_key_hex TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(user_id, name)
);

-- Artifact signatures (ed25519 over content hash)
CREATE TABLE IF NOT EXISTS artifact_signatures (
    id TEXT PRIMARY KEY NOT NULL,
    artifact_id TEXT NOT NULL REFERENCES artifacts(id) ON DELETE CASCADE,
    signer_key_id TEXT NOT NULL REFERENCES user_signing_keys(id),
    signature_hex TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(artifact_id, signer_key_id)
);

-- Ark package registry (AGNOS native packages)
CREATE TABLE IF NOT EXISTS ark_packages (
    id TEXT PRIMARY KEY NOT NULL,
    artifact_id TEXT NOT NULL REFERENCES artifacts(id) ON DELETE CASCADE,
    repo_id TEXT NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    publisher_id TEXT NOT NULL REFERENCES users(id),
    package_name TEXT NOT NULL,
    version TEXT NOT NULL,
    arch TEXT NOT NULL DEFAULT 'any',
    description TEXT,
    dependencies TEXT,  -- JSON array of {"name":"...","version_req":"..."}
    provides TEXT,      -- JSON array of capability strings
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(package_name, version, arch)
);
CREATE INDEX IF NOT EXISTS idx_ark_packages_name ON ark_packages(package_name);
CREATE INDEX IF NOT EXISTS idx_ark_packages_repo ON ark_packages(repo_id);

-- OCI container image manifests
CREATE TABLE IF NOT EXISTS oci_manifests (
    id TEXT PRIMARY KEY NOT NULL,
    repo_id TEXT NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    digest TEXT NOT NULL,          -- sha256:...
    media_type TEXT NOT NULL,
    content_hash TEXT NOT NULL,    -- BLAKE3 hash in BlobStore
    size_bytes INTEGER NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(repo_id, digest)
);
CREATE INDEX IF NOT EXISTS idx_oci_manifests_repo ON oci_manifests(repo_id);

-- OCI tags (mutable pointers to manifests)
CREATE TABLE IF NOT EXISTS oci_tags (
    id TEXT PRIMARY KEY NOT NULL,
    repo_id TEXT NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    tag TEXT NOT NULL,
    manifest_id TEXT NOT NULL REFERENCES oci_manifests(id) ON DELETE CASCADE,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(repo_id, tag)
);

-- OCI blob-to-repo mapping (tracks which blobs belong to which repo)
CREATE TABLE IF NOT EXISTS oci_repo_blobs (
    id TEXT PRIMARY KEY NOT NULL,
    repo_id TEXT NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    digest TEXT NOT NULL,          -- sha256:...
    content_hash TEXT NOT NULL,    -- BLAKE3 hash in BlobStore
    size_bytes INTEGER NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(repo_id, digest)
);

-- OCI chunked upload sessions
CREATE TABLE IF NOT EXISTS oci_blob_uploads (
    id TEXT PRIMARY KEY NOT NULL,  -- upload UUID
    repo_id TEXT NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    state TEXT NOT NULL DEFAULT 'uploading',
    offset_bytes INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
