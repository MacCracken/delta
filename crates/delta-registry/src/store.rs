//! Content-addressable artifact storage.
//!
//! Artifacts are stored by their BLAKE3 hash, enabling deduplication
//! and integrity verification.

use std::path::PathBuf;

/// Content-addressable file store backed by the filesystem.
pub struct BlobStore {
    base_dir: PathBuf,
}

impl BlobStore {
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            base_dir: base_dir.into(),
        }
    }

    /// Store bytes, returning the BLAKE3 content hash.
    pub fn store(&self, data: &[u8]) -> std::io::Result<String> {
        let hash = blake3::hash(data).to_hex().to_string();
        let path = self.blob_path(&hash);

        if path.exists() {
            return Ok(hash); // Deduplicated
        }

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        std::fs::write(&path, data)?;
        Ok(hash)
    }

    /// Read bytes by content hash.
    pub fn read(&self, hash: &str) -> std::io::Result<Vec<u8>> {
        std::fs::read(self.blob_path(hash))
    }

    /// Check if a blob exists.
    pub fn exists(&self, hash: &str) -> bool {
        self.blob_path(hash).exists()
    }

    /// Delete a blob by hash.
    pub fn delete(&self, hash: &str) -> std::io::Result<()> {
        let path = self.blob_path(hash);
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        Ok(())
    }

    /// Get the size of a blob.
    pub fn size(&self, hash: &str) -> std::io::Result<u64> {
        Ok(std::fs::metadata(self.blob_path(hash))?.len())
    }

    /// Path for a blob — uses first 2 chars of hash as directory prefix
    /// to avoid too many files in one directory.
    fn blob_path(&self, hash: &str) -> PathBuf {
        let (prefix, rest) = hash.split_at(2.min(hash.len()));
        self.base_dir.join(prefix).join(rest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_store_and_read() {
        let tmp = tempfile::tempdir().unwrap();
        let store = BlobStore::new(tmp.path());

        let data = b"hello world";
        let hash = store.store(data).unwrap();
        assert!(!hash.is_empty());
        assert!(store.exists(&hash));

        let read_back = store.read(&hash).unwrap();
        assert_eq!(read_back, data);
    }

    #[test]
    fn test_deduplication() {
        let tmp = tempfile::tempdir().unwrap();
        let store = BlobStore::new(tmp.path());

        let data = b"same content";
        let hash1 = store.store(data).unwrap();
        let hash2 = store.store(data).unwrap();
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_delete() {
        let tmp = tempfile::tempdir().unwrap();
        let store = BlobStore::new(tmp.path());

        let hash = store.store(b"temp data").unwrap();
        assert!(store.exists(&hash));

        store.delete(&hash).unwrap();
        assert!(!store.exists(&hash));
    }

    #[test]
    fn test_integrity() {
        let tmp = tempfile::tempdir().unwrap();
        let store = BlobStore::new(tmp.path());

        let data = b"verify me";
        let hash = store.store(data).unwrap();

        // Verify the hash matches
        let expected = blake3::hash(data).to_hex().to_string();
        assert_eq!(hash, expected);
    }
}
