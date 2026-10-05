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
        let path = self.blob_path(&hash)?;

        if path.exists() {
            return Ok(hash); // Deduplicated
        }

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        write_atomically(&path, data)?;
        Ok(hash)
    }

    /// Read bytes by content hash.
    pub fn read(&self, hash: &str) -> std::io::Result<Vec<u8>> {
        std::fs::read(self.blob_path(hash)?)
    }

    /// Check if a blob exists.
    pub fn exists(&self, hash: &str) -> bool {
        self.blob_path(hash).map(|p| p.exists()).unwrap_or(false)
    }

    /// Delete a blob by hash. Blobs are shared by identical content, so
    /// callers should use [`release_blob`] unless they know it is unused.
    pub fn delete(&self, hash: &str) -> std::io::Result<()> {
        let path = self.blob_path(hash)?;
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        Ok(())
    }

    /// Get the size of a blob.
    pub fn size(&self, hash: &str) -> std::io::Result<u64> {
        Ok(std::fs::metadata(self.blob_path(hash)?)?.len())
    }

    /// Path for a blob — uses first 2 chars of hash as directory prefix
    /// to avoid too many files in one directory.
    fn blob_path(&self, hash: &str) -> std::io::Result<PathBuf> {
        if hash.is_empty() || !hash.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "invalid blob hash",
            ));
        }
        let (prefix, rest) = hash.split_at(2.min(hash.len()));
        Ok(self.base_dir.join(prefix).join(rest))
    }
}

/// Delete the blob `hash` unless another artifact, OCI blob, or OCI manifest
/// still references it. Call after removing the row that referenced it.
pub async fn release_blob(
    pool: &sqlx::SqlitePool,
    store: &BlobStore,
    hash: &str,
) -> delta_core::Result<()> {
    if delta_core::db::artifact::blob_is_referenced(pool, hash).await? {
        return Ok(());
    }
    store.delete(hash)?;
    Ok(())
}

/// Write `data` to a temporary file next to `path` and rename it into
/// place, so a crash or full disk never leaves a truncated file at a
/// content address (which deduplication would then trust forever).
pub(crate) fn write_atomically(path: &std::path::Path, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let tmp = path.with_extension(format!("tmp-{}", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(data)?;
        file.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_release_blob_keeps_blobs_still_referenced() {
        use delta_core::db;
        let tmp = tempfile::tempdir().unwrap();
        let url = format!("sqlite://{}", tmp.path().join("delta.db").display());
        let pool = db::init_pool(&url).await.unwrap();
        let store = BlobStore::new(tmp.path().join("blobs"));

        let user = db::user::create(&pool, "alice", "a@example.com", "pw", false)
            .await
            .unwrap();
        let hash = store.store(b"shared bytes").unwrap();
        let mut ids = Vec::new();
        for repo_name in ["one", "two"] {
            let repo = db::repo::create(
                &pool,
                &user.id.to_string(),
                repo_name,
                None,
                delta_core::models::repo::Visibility::Public,
            )
            .await
            .unwrap();
            let artifact = db::artifact::create(
                &pool,
                &db::artifact::CreateArtifactParams {
                    repo_id: &repo.id.to_string(),
                    pipeline_id: None,
                    name: "a",
                    version: None,
                    artifact_type: "generic",
                    content_hash: &hash,
                    size_bytes: 12,
                    metadata: None,
                },
            )
            .await
            .unwrap();
            ids.push(artifact.id);
        }

        // Deleting one copy must not destroy the other repository's artifact.
        db::artifact::delete(&pool, &ids[0]).await.unwrap();
        release_blob(&pool, &store, &hash).await.unwrap();
        assert!(store.exists(&hash));

        db::artifact::delete(&pool, &ids[1]).await.unwrap();
        release_blob(&pool, &store, &hash).await.unwrap();
        assert!(!store.exists(&hash));
    }

    #[test]
    fn test_store_leaves_no_temp_files() {
        let tmp = tempfile::tempdir().unwrap();
        let store = BlobStore::new(tmp.path());
        let hash = store.store(b"data").unwrap();
        let dir = store
            .blob_path(&hash)
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        let names: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names.len(), 1, "{names:?}");
    }

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
