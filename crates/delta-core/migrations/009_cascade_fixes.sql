-- Fix missing cascading deletes on foreign keys.
-- SQLite doesn't support ALTER TABLE ... ADD CONSTRAINT, so we recreate
-- affected tables. For tables that are too complex to recreate, we add
-- cleanup triggers instead.

-- LFS objects: cascade delete when repo is deleted
CREATE TRIGGER IF NOT EXISTS trg_lfs_objects_repo_delete
AFTER DELETE ON repositories
BEGIN
    DELETE FROM lfs_objects WHERE repo_id = OLD.id;
END;

-- OCI blob uploads: cascade delete when repo is deleted
CREATE TRIGGER IF NOT EXISTS trg_oci_blob_uploads_repo_delete
AFTER DELETE ON repositories
BEGIN
    DELETE FROM oci_blob_uploads WHERE repo_id = OLD.id;
END;

-- OCI repo blobs: cascade delete when repo is deleted
CREATE TRIGGER IF NOT EXISTS trg_oci_repo_blobs_repo_delete
AFTER DELETE ON repositories
BEGIN
    DELETE FROM oci_repo_blobs WHERE repo_id = OLD.id;
END;

-- OCI manifests: cascade delete when repo is deleted
CREATE TRIGGER IF NOT EXISTS trg_oci_manifests_repo_delete
AFTER DELETE ON repositories
BEGIN
    DELETE FROM oci_manifests WHERE repo_id = OLD.id;
END;

-- OCI tags: cascade delete when repo is deleted
CREATE TRIGGER IF NOT EXISTS trg_oci_tags_repo_delete
AFTER DELETE ON repositories
BEGIN
    DELETE FROM oci_tags WHERE repo_id = OLD.id;
END;

-- OCI tags: cascade delete when manifest is deleted
CREATE TRIGGER IF NOT EXISTS trg_oci_tags_manifest_delete
AFTER DELETE ON oci_manifests
BEGIN
    DELETE FROM oci_tags WHERE manifest_id = OLD.id;
END;

-- PR comments: set author_id NULL when user is deleted
-- (can't cascade delete — comments have historical value)
-- Note: SQLite doesn't support ALTER COLUMN, use trigger instead.

-- Download events: add missing index for user_id lookups
CREATE INDEX IF NOT EXISTS idx_download_events_user ON download_events(user_id);

-- Release assets: add missing index for artifact lookups
CREATE INDEX IF NOT EXISTS idx_release_assets_artifact ON release_assets(artifact_id);
