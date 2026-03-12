//! OCI container registry helpers.

use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::BlobStore;

/// Compute the sha256 digest of data, returning "sha256:<hex>" format.
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

    fn staging_path(&self, upload_id: &str) -> PathBuf {
        self.staging_dir.join(upload_id)
    }

    /// Append a chunk to the staging file for the given upload.
    pub fn append_chunk(&self, upload_id: &str, data: &[u8]) -> std::io::Result<u64> {
        std::fs::create_dir_all(&self.staging_dir)?;
        let path = self.staging_path(upload_id);
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
        let path = self.staging_path(upload_id);
        let data = std::fs::read(&path)
            .map_err(|e| format!("failed to read staging file: {}", e))?;

        // Verify sha256 digest
        let actual_digest = sha256_digest(&data);
        if actual_digest != expected_digest {
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
        if actual_digest != expected_digest {
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
        let _ = std::fs::remove_file(self.staging_path(upload_id));
    }
}
