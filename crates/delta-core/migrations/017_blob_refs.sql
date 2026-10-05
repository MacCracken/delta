-- Stored blobs are content-addressed and shared between rows; these indexes
-- let deletions check whether a blob is still referenced.
CREATE INDEX IF NOT EXISTS idx_artifacts_content_hash ON artifacts(content_hash);
CREATE INDEX IF NOT EXISTS idx_oci_repo_blobs_content_hash ON oci_repo_blobs(content_hash);
CREATE INDEX IF NOT EXISTS idx_oci_manifests_content_hash ON oci_manifests(content_hash);
