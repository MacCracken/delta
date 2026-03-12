use sqlx::SqlitePool;

use crate::Result;
use crate::models::lfs::LfsObject;

#[derive(sqlx::FromRow)]
struct LfsRow {
    id: String,
    repo_id: String,
    oid: String,
    size: i64,
    created_at: String,
}

impl LfsRow {
    fn into_object(self) -> LfsObject {
        LfsObject {
            id: self.id.parse().unwrap_or_default(),
            repo_id: self.repo_id.parse().unwrap_or_default(),
            oid: self.oid,
            size: self.size,
            created_at: self.created_at.parse().unwrap_or_default(),
        }
    }
}

/// Record an LFS object in the database.
pub async fn create(pool: &SqlitePool, repo_id: &str, oid: &str, size: i64) -> Result<LfsObject> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();

    sqlx::query(
        "INSERT INTO lfs_objects (id, repo_id, oid, size, created_at) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(repo_id)
    .bind(oid)
    .bind(size)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(|e| {
        if e.to_string().contains("UNIQUE") {
            crate::DeltaError::Conflict("LFS object already exists".into())
        } else {
            crate::DeltaError::Storage(e.to_string())
        }
    })?;

    get(pool, repo_id, oid).await
}

/// Get an LFS object by repo and OID.
pub async fn get(pool: &SqlitePool, repo_id: &str, oid: &str) -> Result<LfsObject> {
    let row = sqlx::query_as::<_, LfsRow>(
        "SELECT id, repo_id, oid, size, created_at FROM lfs_objects WHERE repo_id = ? AND oid = ?",
    )
    .bind(repo_id)
    .bind(oid)
    .fetch_optional(pool)
    .await
    .map_err(|e| crate::DeltaError::Storage(e.to_string()))?
    .ok_or_else(|| crate::DeltaError::Storage("LFS object not found".into()))?;

    Ok(row.into_object())
}

/// Check if an LFS object exists for a repo.
pub async fn exists(pool: &SqlitePool, repo_id: &str, oid: &str) -> Result<bool> {
    let count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM lfs_objects WHERE repo_id = ? AND oid = ?",
    )
    .bind(repo_id)
    .bind(oid)
    .fetch_one(pool)
    .await
    .map_err(|e| crate::DeltaError::Storage(e.to_string()))?;

    Ok(count > 0)
}

/// List LFS objects for a repository.
pub async fn list_by_repo(pool: &SqlitePool, repo_id: &str) -> Result<Vec<LfsObject>> {
    let rows = sqlx::query_as::<_, LfsRow>(
        "SELECT id, repo_id, oid, size, created_at FROM lfs_objects WHERE repo_id = ? ORDER BY created_at DESC",
    )
    .bind(repo_id)
    .fetch_all(pool)
    .await
    .map_err(|e| crate::DeltaError::Storage(e.to_string()))?;

    Ok(rows.into_iter().map(LfsRow::into_object).collect())
}

/// Delete an LFS object record.
pub async fn delete(pool: &SqlitePool, repo_id: &str, oid: &str) -> Result<()> {
    sqlx::query("DELETE FROM lfs_objects WHERE repo_id = ? AND oid = ?")
        .bind(repo_id)
        .bind(oid)
        .execute(pool)
        .await
        .map_err(|e| crate::DeltaError::Storage(e.to_string()))?;

    Ok(())
}
