use crate::{DeltaError, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArkPackage {
    pub id: String,
    pub artifact_id: String,
    pub repo_id: String,
    pub publisher_id: String,
    pub package_name: String,
    pub version: String,
    pub arch: String,
    pub description: Option<String>,
    pub dependencies: Option<String>,
    pub provides: Option<String>,
    pub created_at: String,
}

#[derive(sqlx::FromRow)]
struct ArkPackageRow {
    id: String,
    artifact_id: String,
    repo_id: String,
    publisher_id: String,
    package_name: String,
    version: String,
    arch: String,
    description: Option<String>,
    dependencies: Option<String>,
    provides: Option<String>,
    created_at: String,
}

impl ArkPackageRow {
    fn into_package(self) -> ArkPackage {
        ArkPackage {
            id: self.id,
            artifact_id: self.artifact_id,
            repo_id: self.repo_id,
            publisher_id: self.publisher_id,
            package_name: self.package_name,
            version: self.version,
            arch: self.arch,
            description: self.description,
            dependencies: self.dependencies,
            provides: self.provides,
            created_at: self.created_at,
        }
    }
}

pub struct PublishParams<'a> {
    pub artifact_id: &'a str,
    pub repo_id: &'a str,
    pub publisher_id: &'a str,
    pub package_name: &'a str,
    pub version: &'a str,
    pub arch: &'a str,
    pub description: Option<&'a str>,
    pub dependencies: Option<&'a str>,
    pub provides: Option<&'a str>,
}

pub async fn publish(pool: &SqlitePool, params: &PublishParams<'_>) -> Result<ArkPackage> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();

    sqlx::query(
        "INSERT INTO ark_packages (id, artifact_id, repo_id, publisher_id, package_name, version, arch, description, dependencies, provides, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(params.artifact_id)
    .bind(params.repo_id)
    .bind(params.publisher_id)
    .bind(params.package_name)
    .bind(params.version)
    .bind(params.arch)
    .bind(params.description)
    .bind(params.dependencies)
    .bind(params.provides)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(|e| {
        if e.to_string().contains("UNIQUE") {
            DeltaError::Conflict(format!(
                "package {}@{} ({}) already exists",
                params.package_name, params.version, params.arch
            ))
        } else {
            DeltaError::Registry(e.to_string())
        }
    })?;

    get(pool, &id).await
}

pub async fn get(pool: &SqlitePool, id: &str) -> Result<ArkPackage> {
    sqlx::query_as::<_, ArkPackageRow>("SELECT * FROM ark_packages WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|e| DeltaError::Registry(e.to_string()))?
        .map(|r| r.into_package())
        .ok_or_else(|| DeltaError::Registry("package not found".into()))
}

pub async fn get_version(
    pool: &SqlitePool,
    name: &str,
    version: &str,
    arch: Option<&str>,
) -> Result<ArkPackage> {
    let arch = arch.unwrap_or("any");
    sqlx::query_as::<_, ArkPackageRow>(
        "SELECT * FROM ark_packages WHERE package_name = ? AND version = ? AND arch = ?",
    )
    .bind(name)
    .bind(version)
    .bind(arch)
    .fetch_optional(pool)
    .await
    .map_err(|e| DeltaError::Registry(e.to_string()))?
    .map(|r| r.into_package())
    .ok_or_else(|| DeltaError::Registry(format!("package {}@{} not found", name, version)))
}

pub async fn list_versions(pool: &SqlitePool, name: &str) -> Result<Vec<ArkPackage>> {
    let rows = sqlx::query_as::<_, ArkPackageRow>(
        "SELECT * FROM ark_packages WHERE package_name = ? ORDER BY created_at DESC",
    )
    .bind(name)
    .fetch_all(pool)
    .await
    .map_err(|e| DeltaError::Registry(e.to_string()))?;
    Ok(rows.into_iter().map(|r| r.into_package()).collect())
}

pub async fn search(
    pool: &SqlitePool,
    query: &str,
    limit: i64,
    offset: i64,
) -> Result<Vec<ArkPackage>> {
    let pattern = format!("%{}%", query);
    let rows = sqlx::query_as::<_, ArkPackageRow>(
        "SELECT * FROM ark_packages WHERE package_name LIKE ? OR description LIKE ?
         ORDER BY created_at DESC LIMIT ? OFFSET ?",
    )
    .bind(&pattern)
    .bind(&pattern)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await
    .map_err(|e| DeltaError::Registry(e.to_string()))?;
    Ok(rows.into_iter().map(|r| r.into_package()).collect())
}

pub async fn delete(pool: &SqlitePool, id: &str) -> Result<()> {
    let result = sqlx::query("DELETE FROM ark_packages WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await
        .map_err(|e| DeltaError::Registry(e.to_string()))?;
    if result.rows_affected() == 0 {
        return Err(DeltaError::Registry("package not found".into()));
    }
    Ok(())
}

pub async fn get_latest(
    pool: &SqlitePool,
    name: &str,
    arch: Option<&str>,
) -> Result<ArkPackage> {
    let row = if let Some(arch) = arch {
        sqlx::query_as::<_, ArkPackageRow>(
            "SELECT * FROM ark_packages WHERE package_name = ? AND arch = ? ORDER BY created_at DESC LIMIT 1",
        )
        .bind(name)
        .bind(arch)
        .fetch_optional(pool)
        .await
    } else {
        sqlx::query_as::<_, ArkPackageRow>(
            "SELECT * FROM ark_packages WHERE package_name = ? ORDER BY created_at DESC LIMIT 1",
        )
        .bind(name)
        .fetch_optional(pool)
        .await
    }
    .map_err(|e| DeltaError::Registry(e.to_string()))?;

    row.map(|r| r.into_package())
        .ok_or_else(|| DeltaError::Registry(format!("package '{}' not found", name)))
}
