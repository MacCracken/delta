use crate::{DeltaError, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

// --- Manifests ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OciManifest {
    pub id: String,
    pub repo_id: String,
    pub digest: String,
    pub media_type: String,
    pub content_hash: String,
    pub size_bytes: i64,
    pub created_at: String,
}

#[derive(sqlx::FromRow)]
struct ManifestRow {
    id: String,
    repo_id: String,
    digest: String,
    media_type: String,
    content_hash: String,
    size_bytes: i64,
    created_at: String,
}

impl ManifestRow {
    fn into_manifest(self) -> OciManifest {
        OciManifest {
            id: self.id,
            repo_id: self.repo_id,
            digest: self.digest,
            media_type: self.media_type,
            content_hash: self.content_hash,
            size_bytes: self.size_bytes,
            created_at: self.created_at,
        }
    }
}

pub async fn upsert_manifest(
    pool: &SqlitePool,
    repo_id: &str,
    digest: &str,
    media_type: &str,
    content_hash: &str,
    size_bytes: i64,
) -> Result<OciManifest> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();

    sqlx::query(
        "INSERT INTO oci_manifests (id, repo_id, digest, media_type, content_hash, size_bytes, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(repo_id, digest) DO UPDATE SET
            media_type = excluded.media_type,
            content_hash = excluded.content_hash,
            size_bytes = excluded.size_bytes",
    )
    .bind(&id)
    .bind(repo_id)
    .bind(digest)
    .bind(media_type)
    .bind(content_hash)
    .bind(size_bytes)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(|e| DeltaError::Registry(e.to_string()))?;

    get_manifest_by_digest(pool, repo_id, digest).await
}

pub async fn get_manifest_by_digest(
    pool: &SqlitePool,
    repo_id: &str,
    digest: &str,
) -> Result<OciManifest> {
    sqlx::query_as::<_, ManifestRow>("SELECT * FROM oci_manifests WHERE repo_id = ? AND digest = ?")
        .bind(repo_id)
        .bind(digest)
        .fetch_optional(pool)
        .await
        .map_err(|e| DeltaError::Registry(e.to_string()))?
        .map(|r| r.into_manifest())
        .ok_or_else(|| DeltaError::Registry("manifest not found".into()))
}

pub async fn get_manifest_by_tag(
    pool: &SqlitePool,
    repo_id: &str,
    tag: &str,
) -> Result<OciManifest> {
    sqlx::query_as::<_, ManifestRow>(
        "SELECT m.* FROM oci_manifests m
         JOIN oci_tags t ON t.manifest_id = m.id
         WHERE t.repo_id = ? AND t.tag = ?",
    )
    .bind(repo_id)
    .bind(tag)
    .fetch_optional(pool)
    .await
    .map_err(|e| DeltaError::Registry(e.to_string()))?
    .map(|r| r.into_manifest())
    .ok_or_else(|| DeltaError::Registry(format!("tag '{}' not found", tag)))
}

pub async fn delete_manifest(pool: &SqlitePool, repo_id: &str, digest: &str) -> Result<()> {
    let result = sqlx::query("DELETE FROM oci_manifests WHERE repo_id = ? AND digest = ?")
        .bind(repo_id)
        .bind(digest)
        .execute(pool)
        .await
        .map_err(|e| DeltaError::Registry(e.to_string()))?;
    if result.rows_affected() == 0 {
        return Err(DeltaError::Registry("manifest not found".into()));
    }
    Ok(())
}

// --- Tags ---

pub async fn put_tag(pool: &SqlitePool, repo_id: &str, tag: &str, manifest_id: &str) -> Result<()> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();

    sqlx::query(
        "INSERT INTO oci_tags (id, repo_id, tag, manifest_id, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?)
         ON CONFLICT(repo_id, tag) DO UPDATE SET
            manifest_id = excluded.manifest_id,
            updated_at = excluded.updated_at",
    )
    .bind(&id)
    .bind(repo_id)
    .bind(tag)
    .bind(manifest_id)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(|e| DeltaError::Registry(e.to_string()))?;

    Ok(())
}

pub async fn list_tags(pool: &SqlitePool, repo_id: &str) -> Result<Vec<String>> {
    let rows: Vec<(String,)> =
        sqlx::query_as("SELECT tag FROM oci_tags WHERE repo_id = ? ORDER BY updated_at DESC")
            .bind(repo_id)
            .fetch_all(pool)
            .await
            .map_err(|e| DeltaError::Registry(e.to_string()))?;
    Ok(rows.into_iter().map(|r| r.0).collect())
}

// --- Repo Blobs ---

pub async fn upsert_repo_blob(
    pool: &SqlitePool,
    repo_id: &str,
    digest: &str,
    content_hash: &str,
    size_bytes: i64,
) -> Result<()> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();

    sqlx::query(
        "INSERT INTO oci_repo_blobs (id, repo_id, digest, content_hash, size_bytes, created_at)
         VALUES (?, ?, ?, ?, ?, ?)
         ON CONFLICT(repo_id, digest) DO UPDATE SET
            content_hash = excluded.content_hash,
            size_bytes = excluded.size_bytes",
    )
    .bind(&id)
    .bind(repo_id)
    .bind(digest)
    .bind(content_hash)
    .bind(size_bytes)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(|e| DeltaError::Registry(e.to_string()))?;

    Ok(())
}

pub async fn get_repo_blob(pool: &SqlitePool, repo_id: &str, digest: &str) -> Result<OciRepoBlob> {
    sqlx::query_as::<_, RepoBlobRow>(
        "SELECT * FROM oci_repo_blobs WHERE repo_id = ? AND digest = ?",
    )
    .bind(repo_id)
    .bind(digest)
    .fetch_optional(pool)
    .await
    .map_err(|e| DeltaError::Registry(e.to_string()))?
    .map(|r| OciRepoBlob {
        digest: r.digest,
        content_hash: r.content_hash,
        size_bytes: r.size_bytes,
    })
    .ok_or_else(|| DeltaError::Registry("blob not found".into()))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OciRepoBlob {
    pub digest: String,
    pub content_hash: String,
    pub size_bytes: i64,
}

#[derive(sqlx::FromRow)]
struct RepoBlobRow {
    #[allow(dead_code)]
    id: String,
    #[allow(dead_code)]
    repo_id: String,
    digest: String,
    content_hash: String,
    size_bytes: i64,
    #[allow(dead_code)]
    created_at: String,
}

pub async fn delete_repo_blob(pool: &SqlitePool, repo_id: &str, digest: &str) -> Result<()> {
    let result = sqlx::query("DELETE FROM oci_repo_blobs WHERE repo_id = ? AND digest = ?")
        .bind(repo_id)
        .bind(digest)
        .execute(pool)
        .await
        .map_err(|e| DeltaError::Registry(e.to_string()))?;
    if result.rows_affected() == 0 {
        return Err(DeltaError::Registry("blob not found".into()));
    }
    Ok(())
}

// --- Blob Uploads ---

pub async fn create_blob_upload(pool: &SqlitePool, repo_id: &str) -> Result<String> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();

    sqlx::query(
        "INSERT INTO oci_blob_uploads (id, repo_id, state, offset_bytes, created_at, updated_at)
         VALUES (?, ?, 'uploading', 0, ?, ?)",
    )
    .bind(&id)
    .bind(repo_id)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(|e| DeltaError::Registry(e.to_string()))?;

    Ok(id)
}

#[derive(Debug, Clone)]
pub struct BlobUpload {
    pub id: String,
    pub repo_id: String,
    pub state: String,
    pub offset_bytes: i64,
}

#[derive(sqlx::FromRow)]
struct BlobUploadRow {
    id: String,
    repo_id: String,
    state: String,
    offset_bytes: i64,
    #[allow(dead_code)]
    created_at: String,
    #[allow(dead_code)]
    updated_at: String,
}

pub async fn get_blob_upload(pool: &SqlitePool, upload_id: &str) -> Result<BlobUpload> {
    sqlx::query_as::<_, BlobUploadRow>("SELECT * FROM oci_blob_uploads WHERE id = ?")
        .bind(upload_id)
        .fetch_optional(pool)
        .await
        .map_err(|e| DeltaError::Registry(e.to_string()))?
        .map(|r| BlobUpload {
            id: r.id,
            repo_id: r.repo_id,
            state: r.state,
            offset_bytes: r.offset_bytes,
        })
        .ok_or_else(|| DeltaError::Registry("upload not found".into()))
}

pub async fn update_blob_upload_offset(
    pool: &SqlitePool,
    upload_id: &str,
    new_offset: i64,
) -> Result<()> {
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "UPDATE oci_blob_uploads SET offset_bytes = ?, updated_at = ? WHERE id = ? AND state = 'uploading'",
    )
    .bind(new_offset)
    .bind(&now)
    .bind(upload_id)
    .execute(pool)
    .await
    .map_err(|e| DeltaError::Registry(e.to_string()))?;
    Ok(())
}

pub async fn complete_blob_upload(pool: &SqlitePool, upload_id: &str) -> Result<()> {
    let now = Utc::now().to_rfc3339();
    sqlx::query("UPDATE oci_blob_uploads SET state = 'completed', updated_at = ? WHERE id = ?")
        .bind(&now)
        .bind(upload_id)
        .execute(pool)
        .await
        .map_err(|e| DeltaError::Registry(e.to_string()))?;
    Ok(())
}

pub async fn delete_blob_upload(pool: &SqlitePool, upload_id: &str) -> Result<()> {
    sqlx::query("DELETE FROM oci_blob_uploads WHERE id = ?")
        .bind(upload_id)
        .execute(pool)
        .await
        .map_err(|e| DeltaError::Registry(e.to_string()))?;
    Ok(())
}
