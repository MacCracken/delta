//! LFS object storage.
//!
//! Git LFS requires SHA-256 content-addressed storage, separate from
//! the BLAKE3-based BlobStore used for artifacts.

use sha2::{Digest, Sha256};
use std::path::PathBuf;

/// SHA-256 content-addressed store for LFS objects.
pub struct LfsStore {
    base_dir: PathBuf,
}

impl LfsStore {
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            base_dir: base_dir.into(),
        }
    }

    /// Store bytes, returning the SHA-256 hex hash.
    pub fn store(&self, data: &[u8]) -> std::io::Result<String> {
        let hash = hex_sha256(data);
        let path = self.object_path(&hash)?;

        if path.exists() {
            return Ok(hash);
        }

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        std::fs::write(&path, data)?;
        Ok(hash)
    }

    /// Store data and verify it matches the expected OID.
    pub fn store_verified(&self, data: &[u8], expected_oid: &str) -> std::io::Result<()> {
        let hash = hex_sha256(data);
        if !constant_time_eq(hash.as_bytes(), expected_oid.as_bytes()) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("SHA-256 mismatch: expected {}, got {}", expected_oid, hash),
            ));
        }

        let path = self.object_path(&hash)?;
        if path.exists() {
            return Ok(());
        }

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        std::fs::write(&path, data)?;
        Ok(())
    }

    /// Read an LFS object by OID.
    pub fn read(&self, oid: &str) -> std::io::Result<Vec<u8>> {
        std::fs::read(self.object_path(oid)?)
    }

    /// Check if an object exists on disk.
    pub fn exists(&self, oid: &str) -> bool {
        self.object_path(oid).map(|p| p.exists()).unwrap_or(false)
    }

    /// Delete an object by OID.
    pub fn delete(&self, oid: &str) -> std::io::Result<()> {
        let path = self.object_path(oid)?;
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        Ok(())
    }

    /// Get the size of an object on disk.
    pub fn size(&self, oid: &str) -> std::io::Result<u64> {
        Ok(std::fs::metadata(self.object_path(oid)?)?.len())
    }

    /// Object path: lfs/{oid[0..2]}/{oid[2..4]}/{oid}
    /// Uses two levels of prefix sharding per the LFS storage convention.
    fn object_path(&self, oid: &str) -> std::io::Result<PathBuf> {
        if oid.len() < 4 || !oid.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "invalid LFS OID",
            ));
        }
        Ok(self.base_dir.join(&oid[..2]).join(&oid[2..4]).join(oid))
    }
}

/// Compute SHA-256 hex digest.
fn hex_sha256(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

/// Validate that an OID looks like a valid SHA-256 hex string.
pub fn validate_oid(oid: &str) -> bool {
    oid.len() == 64 && oid.chars().all(|c| c.is_ascii_hexdigit())
}

/// Constant-time byte comparison to prevent timing attacks on hash verification.
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
    fn test_store_and_read() {
        let tmp = tempfile::tempdir().unwrap();
        let store = LfsStore::new(tmp.path());

        let data = b"hello lfs world";
        let oid = store.store(data).unwrap();
        assert_eq!(oid.len(), 64);
        assert!(store.exists(&oid));

        let read_back = store.read(&oid).unwrap();
        assert_eq!(read_back, data);
    }

    #[test]
    fn test_store_verified_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let store = LfsStore::new(tmp.path());

        let data = b"verified content";
        let oid = hex_sha256(data);
        store.store_verified(data, &oid).unwrap();
        assert!(store.exists(&oid));
    }

    #[test]
    fn test_store_verified_mismatch() {
        let tmp = tempfile::tempdir().unwrap();
        let store = LfsStore::new(tmp.path());

        let data = b"some content";
        let wrong_oid = "a".repeat(64);
        let err = store.store_verified(data, &wrong_oid).unwrap_err();
        assert!(err.to_string().contains("SHA-256 mismatch"));
    }

    #[test]
    fn test_deduplication() {
        let tmp = tempfile::tempdir().unwrap();
        let store = LfsStore::new(tmp.path());

        let data = b"duplicate me";
        let oid1 = store.store(data).unwrap();
        let oid2 = store.store(data).unwrap();
        assert_eq!(oid1, oid2);
    }

    #[test]
    fn test_delete() {
        let tmp = tempfile::tempdir().unwrap();
        let store = LfsStore::new(tmp.path());

        let oid = store.store(b"temp").unwrap();
        assert!(store.exists(&oid));
        store.delete(&oid).unwrap();
        assert!(!store.exists(&oid));
    }

    #[test]
    fn test_validate_oid() {
        assert!(validate_oid(
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        ));
        assert!(!validate_oid("too-short"));
        assert!(!validate_oid(""));
        assert!(!validate_oid(&"g".repeat(64))); // non-hex
    }

    #[test]
    fn test_hex_sha256() {
        // SHA-256 of empty string
        let hash = hex_sha256(b"");
        assert_eq!(
            hash,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }
}
