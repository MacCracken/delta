use crate::{DeltaError, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Release {
    pub id: String,
    pub repo_id: String,
    pub tag_name: String,
    pub name: String,
    pub body: Option<String>,
    pub is_draft: bool,
    pub is_prerelease: bool,
    pub author_id: String,
    pub created_at: String,
}

pub struct CreateReleaseParams<'a> {
    pub repo_id: &'a str,
    pub tag_name: &'a str,
    pub name: &'a str,
    pub body: Option<&'a str>,
    pub is_draft: bool,
    pub is_prerelease: bool,
    pub author_id: &'a str,
}

pub async fn create(pool: &SqlitePool, params: &CreateReleaseParams<'_>) -> Result<Release> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();

    sqlx::query(
        "INSERT INTO releases (id, repo_id, tag_name, name, body, is_draft, is_prerelease, author_id, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(params.repo_id)
    .bind(params.tag_name)
    .bind(params.name)
    .bind(params.body)
    .bind(params.is_draft)
    .bind(params.is_prerelease)
    .bind(params.author_id)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(|e| {
        if e.to_string().contains("UNIQUE") {
            DeltaError::Conflict(format!("release for tag '{}' already exists", params.tag_name))
        } else {
            DeltaError::Registry(e.to_string())
        }
    })?;

    get(pool, &id).await
}

pub async fn get(pool: &SqlitePool, id: &str) -> Result<Release> {
    sqlx::query_as::<_, ReleaseRow>("SELECT * FROM releases WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|e| DeltaError::Registry(e.to_string()))?
        .map(|r| r.into_release())
        .ok_or_else(|| DeltaError::Registry("release not found".into()))
}

pub async fn get_by_tag(pool: &SqlitePool, repo_id: &str, tag_name: &str) -> Result<Release> {
    sqlx::query_as::<_, ReleaseRow>("SELECT * FROM releases WHERE repo_id = ? AND tag_name = ?")
        .bind(repo_id)
        .bind(tag_name)
        .fetch_optional(pool)
        .await
        .map_err(|e| DeltaError::Registry(e.to_string()))?
        .map(|r| r.into_release())
        .ok_or_else(|| DeltaError::Registry(format!("release '{}' not found", tag_name)))
}

pub async fn list_for_repo(pool: &SqlitePool, repo_id: &str) -> Result<Vec<Release>> {
    let rows = sqlx::query_as::<_, ReleaseRow>(
        "SELECT * FROM releases WHERE repo_id = ? ORDER BY created_at DESC",
    )
    .bind(repo_id)
    .fetch_all(pool)
    .await
    .map_err(|e| DeltaError::Registry(e.to_string()))?;
    Ok(rows.into_iter().map(|r| r.into_release()).collect())
}

pub async fn delete(pool: &SqlitePool, id: &str) -> Result<()> {
    let result = sqlx::query("DELETE FROM releases WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await
        .map_err(|e| DeltaError::Registry(e.to_string()))?;
    if result.rows_affected() == 0 {
        return Err(DeltaError::Registry("release not found".into()));
    }
    Ok(())
}

pub async fn attach_asset(
    pool: &SqlitePool,
    release_id: &str,
    artifact_id: &str,
    label: Option<&str>,
) -> Result<()> {
    let id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO release_assets (id, release_id, artifact_id, label) VALUES (?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(release_id)
    .bind(artifact_id)
    .bind(label)
    .execute(pool)
    .await
    .map_err(|e| DeltaError::Registry(e.to_string()))?;
    Ok(())
}

#[derive(sqlx::FromRow)]
struct ReleaseRow {
    id: String,
    repo_id: String,
    tag_name: String,
    name: String,
    body: Option<String>,
    is_draft: bool,
    is_prerelease: bool,
    author_id: String,
    created_at: String,
}

impl ReleaseRow {
    fn into_release(self) -> Release {
        Release {
            id: self.id,
            repo_id: self.repo_id,
            tag_name: self.tag_name,
            name: self.name,
            body: self.body,
            is_draft: self.is_draft,
            is_prerelease: self.is_prerelease,
            author_id: self.author_id,
            created_at: self.created_at,
        }
    }
}
