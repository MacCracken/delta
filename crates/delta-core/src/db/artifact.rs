use crate::{DeltaError, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    pub id: String,
    pub repo_id: String,
    pub pipeline_id: Option<String>,
    pub name: String,
    pub version: Option<String>,
    pub artifact_type: String,
    pub content_hash: String,
    pub size_bytes: i64,
    pub metadata: Option<String>,
    pub download_count: i64,
    pub created_at: String,
}

pub struct CreateArtifactParams<'a> {
    pub repo_id: &'a str,
    pub pipeline_id: Option<&'a str>,
    pub name: &'a str,
    pub version: Option<&'a str>,
    pub artifact_type: &'a str,
    pub content_hash: &'a str,
    pub size_bytes: i64,
    pub metadata: Option<&'a str>,
}

pub async fn create(pool: &SqlitePool, params: &CreateArtifactParams<'_>) -> Result<Artifact> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();

    sqlx::query(
        "INSERT INTO artifacts (id, repo_id, pipeline_id, name, version, artifact_type, content_hash, size_bytes, metadata, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(params.repo_id)
    .bind(params.pipeline_id)
    .bind(params.name)
    .bind(params.version)
    .bind(params.artifact_type)
    .bind(params.content_hash)
    .bind(params.size_bytes)
    .bind(params.metadata)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(|e| DeltaError::Registry(e.to_string()))?;

    get(pool, &id).await
}

pub async fn get(pool: &SqlitePool, id: &str) -> Result<Artifact> {
    sqlx::query_as::<_, ArtifactRow>("SELECT * FROM artifacts WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|e| DeltaError::Registry(e.to_string()))?
        .map(|r| r.into_artifact())
        .ok_or_else(|| DeltaError::Registry("artifact not found".into()))
}

pub async fn list_for_repo(pool: &SqlitePool, repo_id: &str) -> Result<Vec<Artifact>> {
    let rows = sqlx::query_as::<_, ArtifactRow>(
        "SELECT * FROM artifacts WHERE repo_id = ? ORDER BY created_at DESC",
    )
    .bind(repo_id)
    .fetch_all(pool)
    .await
    .map_err(|e| DeltaError::Registry(e.to_string()))?;
    Ok(rows.into_iter().map(|r| r.into_artifact()).collect())
}

pub async fn increment_download(pool: &SqlitePool, id: &str) -> Result<()> {
    sqlx::query("UPDATE artifacts SET download_count = download_count + 1 WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await
        .map_err(|e| DeltaError::Registry(e.to_string()))?;
    Ok(())
}

pub async fn delete(pool: &SqlitePool, id: &str) -> Result<()> {
    let result = sqlx::query("DELETE FROM artifacts WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await
        .map_err(|e| DeltaError::Registry(e.to_string()))?;
    if result.rows_affected() == 0 {
        return Err(DeltaError::Registry("artifact not found".into()));
    }
    Ok(())
}

#[derive(sqlx::FromRow)]
pub(crate) struct ArtifactRowPub {
    pub id: String,
    pub repo_id: String,
    pub pipeline_id: Option<String>,
    pub name: String,
    pub version: Option<String>,
    pub artifact_type: String,
    pub content_hash: String,
    pub size_bytes: i64,
    pub metadata: Option<String>,
    pub download_count: i64,
    pub created_at: String,
}

impl ArtifactRowPub {
    pub fn into_artifact(self) -> Artifact {
        Artifact {
            id: self.id,
            repo_id: self.repo_id,
            pipeline_id: self.pipeline_id,
            name: self.name,
            version: self.version,
            artifact_type: self.artifact_type,
            content_hash: self.content_hash,
            size_bytes: self.size_bytes,
            metadata: self.metadata,
            download_count: self.download_count,
            created_at: self.created_at,
        }
    }
}

#[derive(sqlx::FromRow)]
struct ArtifactRow {
    id: String,
    repo_id: String,
    pipeline_id: Option<String>,
    name: String,
    version: Option<String>,
    artifact_type: String,
    content_hash: String,
    size_bytes: i64,
    metadata: Option<String>,
    download_count: i64,
    created_at: String,
}

impl ArtifactRow {
    fn into_artifact(self) -> Artifact {
        Artifact {
            id: self.id,
            repo_id: self.repo_id,
            pipeline_id: self.pipeline_id,
            name: self.name,
            version: self.version,
            artifact_type: self.artifact_type,
            content_hash: self.content_hash,
            size_bytes: self.size_bytes,
            metadata: self.metadata,
            download_count: self.download_count,
            created_at: self.created_at,
        }
    }
}
