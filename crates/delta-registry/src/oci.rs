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
        let path = self.validated_staging_path(upload_id).map_err(|e| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, e)
        })?;
        std::fs::create_dir_all(&self.staging_dir)?;
        // Check current size before appending
        let current_size = if path.exists() {
            std::fs::metadata(&path)?.len()
        } else {
            0
        };
        if current_size + data.len() as u64 > Self::MAX_STAGING_SIZE {
            return Err(std::io::Error::other(
                format!("upload exceeds maximum size of {} bytes", Self::MAX_STAGING_SIZE),
            ));
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
        let path = self.validated_staging_path(upload_id)
            .map_err(|e| format!("invalid upload ID: {}", e))?;
        let data = std::fs::read(&path)
            .map_err(|e| format!("failed to read staging file: {}", e))?;

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
