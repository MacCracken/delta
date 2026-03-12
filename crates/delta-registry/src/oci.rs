//! OCI container registry helpers.

use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::BlobStore;

/// Compute the sha256 digest of data, returning `sha256:<hex>` format.
pub fn sha256_digest(data: &[u8]) -> String {
    let hash = Sha256::digest(data);
    format!("sha256:{}", hex::encode(hash))
}

/// Manages chunked upload staging files.
pub struct OciStagingArea {
    staging_dir: PathBuf,
}

impl OciStagingArea {
    pub fn new(artifacts_dir: &Path) -> Self {
        let staging_dir = artifacts_dir.join("_oci_staging");
        Self { staging_dir }
    }

    /// Maximum staging file size (1 GB).
    const MAX_STAGING_SIZE: u64 = 1024 * 1024 * 1024;

    /// Append a chunk to the staging file for the given upload.
    pub fn append_chunk(&self, upload_id: &str, data: &[u8]) -> std::io::Result<u64> {
        let path = self
            .validated_staging_path(upload_id)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
        std::fs::create_dir_all(&self.staging_dir)?;
        // Check current size before appending
        let current_size = if path.exists() {
            std::fs::metadata(&path)?.len()
        } else {
            0
        };
        if current_size + data.len() as u64 > Self::MAX_STAGING_SIZE {
            return Err(std::io::Error::other(format!(
                "upload exceeds maximum size of {} bytes",
                Self::MAX_STAGING_SIZE
            )));
        }
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        file.write_all(data)?;
        Ok(std::fs::metadata(&path)?.len())
    }

    /// Finalize the upload: read staging file, verify sha256, store in BlobStore.
    /// Returns (blake3_content_hash, size_bytes).
    pub fn finalize(
        &self,
        upload_id: &str,
        expected_digest: &str,
        blob_store: &BlobStore,
    ) -> Result<(String, i64), String> {
        let path = self
            .validated_staging_path(upload_id)
            .map_err(|e| format!("invalid upload ID: {}", e))?;
        let data =
            std::fs::read(&path).map_err(|e| format!("failed to read staging file: {}", e))?;

        // Verify sha256 digest (constant-time comparison)
        let actual_digest = sha256_digest(&data);
        if !constant_time_eq(actual_digest.as_bytes(), expected_digest.as_bytes()) {
            let _ = std::fs::remove_file(&path);
            return Err(format!(
                "digest mismatch: expected {}, got {}",
                expected_digest, actual_digest
            ));
        }

        let size = data.len() as i64;
        let content_hash = blob_store
            .store(&data)
            .map_err(|e| format!("failed to store blob: {}", e))?;

        // Clean up staging file
        let _ = std::fs::remove_file(&path);

        Ok((content_hash, size))
    }

    /// Store data directly (monolithic upload), verifying digest.
    /// Returns (blake3_content_hash, size_bytes).
    pub fn store_monolithic(
        &self,
        data: &[u8],
        expected_digest: &str,
        blob_store: &BlobStore,
    ) -> Result<(String, i64), String> {
        let actual_digest = sha256_digest(data);
        if !constant_time_eq(actual_digest.as_bytes(), expected_digest.as_bytes()) {
            return Err(format!(
                "digest mismatch: expected {}, got {}",
                expected_digest, actual_digest
            ));
        }

        let size = data.len() as i64;
        let content_hash = blob_store
            .store(data)
            .map_err(|e| format!("failed to store blob: {}", e))?;

        Ok((content_hash, size))
    }

    /// Clean up a staging file for an abandoned upload.
    pub fn cleanup(&self, upload_id: &str) {
        if let Ok(path) = self.validated_staging_path(upload_id) {
            let _ = std::fs::remove_file(path);
        }
    }

    /// Validate upload_id is a UUID-like string (alphanumeric + hyphens only)
    /// to prevent path traversal.
    fn validated_staging_path(&self, upload_id: &str) -> Result<PathBuf, String> {
        if upload_id.is_empty()
            || upload_id.len() > 64
            || !upload_id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-')
        {
            return Err("invalid upload ID".into());
        }
        Ok(self.staging_dir.join(upload_id))
    }
}

/// Constant-time byte comparison to prevent timing attacks on digest verification.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sha256_digest() {
        let data = b"hello world";
        let digest = sha256_digest(data);
        assert!(digest.starts_with("sha256:"));
        assert_eq!(digest.len(), 7 + 64); // "sha256:" + 64 hex chars
    }

    #[test]
    fn test_sha256_digest_empty() {
        let digest = sha256_digest(b"");
        assert!(digest.starts_with("sha256:"));
        // SHA256 of empty is well-known
        assert_eq!(
            digest,
            "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn test_sha256_digest_deterministic() {
        let d1 = sha256_digest(b"test data");
        let d2 = sha256_digest(b"test data");
        assert_eq!(d1, d2);
    }

    #[test]
    fn test_constant_time_eq_equal() {
        assert!(constant_time_eq(b"hello", b"hello"));
        assert!(constant_time_eq(b"", b""));
        assert!(constant_time_eq(b"\x00\x01", b"\x00\x01"));
    }

    #[test]
    fn test_constant_time_eq_not_equal() {
        assert!(!constant_time_eq(b"hello", b"world"));
        assert!(!constant_time_eq(b"abc", b"abd"));
    }

    #[test]
    fn test_constant_time_eq_different_lengths() {
        assert!(!constant_time_eq(b"short", b"longer string"));
        assert!(!constant_time_eq(b"", b"a"));
    }

    #[test]
    fn test_validated_staging_path_valid() {
        let staging = OciStagingArea::new(std::path::Path::new("/tmp/test"));
        assert!(
            staging
                .validated_staging_path("550e8400-e29b-41d4-a716-446655440000")
                .is_ok()
        );
        assert!(staging.validated_staging_path("abc123").is_ok());
        assert!(staging.validated_staging_path("a-b-c").is_ok());
    }

    #[test]
    fn test_validated_staging_path_rejects_empty() {
        let staging = OciStagingArea::new(std::path::Path::new("/tmp/test"));
        assert!(staging.validated_staging_path("").is_err());
    }

    #[test]
    fn test_validated_staging_path_rejects_traversal() {
        let staging = OciStagingArea::new(std::path::Path::new("/tmp/test"));
        assert!(
            staging
                .validated_staging_path("../../../etc/passwd")
                .is_err()
        );
        assert!(staging.validated_staging_path("..").is_err());
        assert!(staging.validated_staging_path("a/b").is_err());
    }

    #[test]
    fn test_validated_staging_path_rejects_too_long() {
        let staging = OciStagingArea::new(std::path::Path::new("/tmp/test"));
        let long_id = "a".repeat(65);
        assert!(staging.validated_staging_path(&long_id).is_err());

        let max_id = "a".repeat(64);
        assert!(staging.validated_staging_path(&max_id).is_ok());
    }

    #[test]
    fn test_staging_append_and_finalize() {
        let tmp = tempfile::tempdir().unwrap();
        let blob_store = BlobStore::new(tmp.path().join("blobs"));
        let staging = OciStagingArea::new(tmp.path());

        let data = b"hello world";
        let offset = staging.append_chunk("test-upload-1", data).unwrap();
        assert_eq!(offset, 11);

        // Append more
        let offset = staging.append_chunk("test-upload-1", b" again").unwrap();
        assert_eq!(offset, 17);

        // Finalize with correct digest
        let full_data = b"hello world again";
        let digest = sha256_digest(full_data);
        let (content_hash, size) = staging
            .finalize("test-upload-1", &digest, &blob_store)
            .unwrap();
        assert_eq!(size, 17);
        assert!(!content_hash.is_empty());
    }

    #[test]
    fn test_staging_finalize_digest_mismatch() {
        let tmp = tempfile::tempdir().unwrap();
        let blob_store = BlobStore::new(tmp.path().join("blobs"));
        let staging = OciStagingArea::new(tmp.path());

        staging.append_chunk("test-upload-2", b"data").unwrap();

        let result = staging.finalize("test-upload-2", "sha256:wrong", &blob_store);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("digest mismatch"));
    }

    #[test]
    fn test_store_monolithic() {
        let tmp = tempfile::tempdir().unwrap();
        let blob_store = BlobStore::new(tmp.path().join("blobs"));
        let staging = OciStagingArea::new(tmp.path());

        let data = b"monolithic upload";
        let digest = sha256_digest(data);
        let (hash, size) = staging
            .store_monolithic(data, &digest, &blob_store)
            .unwrap();
        assert_eq!(size, 17);
        assert!(!hash.is_empty());
    }

    #[test]
    fn test_store_monolithic_wrong_digest() {
        let tmp = tempfile::tempdir().unwrap();
        let blob_store = BlobStore::new(tmp.path().join("blobs"));
        let staging = OciStagingArea::new(tmp.path());

        let result = staging.store_monolithic(b"data", "sha256:bad", &blob_store);
        assert!(result.is_err());
    }

    #[test]
    fn test_staging_cleanup() {
        let tmp = tempfile::tempdir().unwrap();
        let staging = OciStagingArea::new(tmp.path());

        staging.append_chunk("cleanup-test", b"data").unwrap();
        let path = staging.validated_staging_path("cleanup-test").unwrap();
        assert!(path.exists());

        staging.cleanup("cleanup-test");
        assert!(!path.exists());
    }

    #[test]
    fn test_staging_cleanup_nonexistent() {
        let tmp = tempfile::tempdir().unwrap();
        let staging = OciStagingArea::new(tmp.path());
        // Should not panic
        staging.cleanup("nonexistent");
    }
}
