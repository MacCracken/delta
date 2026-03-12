use crate::models::repo::{Repository, Visibility};
use crate::{DeltaError, Result};
use chrono::Utc;
use sqlx::SqlitePool;
use uuid::Uuid;

/// Create a new repository record.
pub async fn create(
    pool: &SqlitePool,
    owner_id: &str,
    name: &str,
    description: Option<&str>,
    visibility: Visibility,
) -> Result<Repository> {
    let id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();
    let vis = visibility.as_str().to_string();

    sqlx::query(
        "INSERT INTO repositories (id, owner_id, name, description, visibility, default_branch, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, 'main', ?, ?)",
    )
    .bind(&id)
    .bind(owner_id)
    .bind(name)
    .bind(description)
    .bind(&vis)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(|e| {
        if e.to_string().contains("UNIQUE") {
            DeltaError::Conflict(format!("repository '{}' already exists", name))
        } else {
            DeltaError::Storage(e.to_string())
        }
    })?;

    get_by_id(pool, &id).await
}

/// Get a repository by ID.
pub async fn get_by_id(pool: &SqlitePool, id: &str) -> Result<Repository> {
    let row = sqlx::query_as::<_, RepoRow>("SELECT * FROM repositories WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|e| DeltaError::Storage(e.to_string()))?
        .ok_or_else(|| DeltaError::RepoNotFound(id.to_string()))?;

    Ok(row.into_repo())
}

/// Get a repository by owner_id and name.
pub async fn get_by_owner_and_name(
    pool: &SqlitePool,
    owner_id: &str,
    name: &str,
) -> Result<Repository> {
    let row =
        sqlx::query_as::<_, RepoRow>("SELECT * FROM repositories WHERE owner_id = ? AND name = ?")
            .bind(owner_id)
            .bind(name)
            .fetch_optional(pool)
            .await
            .map_err(|e| DeltaError::Storage(e.to_string()))?
            .ok_or_else(|| DeltaError::RepoNotFound(format!("{}/{}", owner_id, name)))?;

    Ok(row.into_repo())
}

/// List repositories for an owner.
pub async fn list_by_owner(pool: &SqlitePool, owner_id: &str) -> Result<Vec<Repository>> {
    let rows = sqlx::query_as::<_, RepoRow>(
        "SELECT * FROM repositories WHERE owner_id = ? ORDER BY updated_at DESC",
    )
    .bind(owner_id)
    .fetch_all(pool)
    .await
    .map_err(|e| DeltaError::Storage(e.to_string()))?;

    Ok(rows.into_iter().map(|r| r.into_repo()).collect())
}

/// List all visible repositories (public + user's own + collaborations).
pub async fn list_visible(pool: &SqlitePool, viewer_id: Option<&str>) -> Result<Vec<Repository>> {
    let rows = if let Some(uid) = viewer_id {
        sqlx::query_as::<_, RepoRow>(
            "SELECT DISTINCT r.* FROM repositories r
             LEFT JOIN repository_collaborators c ON r.id = c.repo_id AND c.user_id = ?
             WHERE r.visibility = 'public' OR r.owner_id = ? OR c.user_id IS NOT NULL
             ORDER BY r.updated_at DESC",
        )
        .bind(uid)
        .bind(uid)
        .fetch_all(pool)
        .await
    } else {
        sqlx::query_as::<_, RepoRow>(
            "SELECT * FROM repositories WHERE visibility = 'public' ORDER BY updated_at DESC",
        )
        .fetch_all(pool)
        .await
    }
    .map_err(|e| DeltaError::Storage(e.to_string()))?;

    Ok(rows.into_iter().map(|r| r.into_repo()).collect())
}

/// Update a repository's metadata.
pub async fn update(
    pool: &SqlitePool,
    id: &str,
    description: Option<&str>,
    visibility: Option<Visibility>,
    default_branch: Option<&str>,
) -> Result<Repository> {
    let now = Utc::now().to_rfc3339();

    if let Some(desc) = description {
        sqlx::query("UPDATE repositories SET description = ?, updated_at = ? WHERE id = ?")
            .bind(desc)
            .bind(&now)
            .bind(id)
            .execute(pool)
            .await
            .map_err(|e| DeltaError::Storage(e.to_string()))?;
    }

    if let Some(vis) = visibility {
        let vis_str = vis.as_str().to_string();
        sqlx::query("UPDATE repositories SET visibility = ?, updated_at = ? WHERE id = ?")
            .bind(&vis_str)
            .bind(&now)
            .bind(id)
            .execute(pool)
            .await
            .map_err(|e| DeltaError::Storage(e.to_string()))?;
    }

    if let Some(branch) = default_branch {
        sqlx::query("UPDATE repositories SET default_branch = ?, updated_at = ? WHERE id = ?")
            .bind(branch)
            .bind(&now)
            .bind(id)
            .execute(pool)
            .await
            .map_err(|e| DeltaError::Storage(e.to_string()))?;
    }

    get_by_id(pool, id).await
}

/// Delete a repository.
pub async fn delete(pool: &SqlitePool, id: &str) -> Result<()> {
    let result = sqlx::query("DELETE FROM repositories WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await
        .map_err(|e| DeltaError::Storage(e.to_string()))?;

    if result.rows_affected() == 0 {
        return Err(DeltaError::RepoNotFound(id.to_string()));
    }
    Ok(())
}

#[derive(sqlx::FromRow)]
struct RepoRow {
    id: String,
    owner_id: String,
    name: String,
    description: Option<String>,
    visibility: String,
    default_branch: String,
    created_at: String,
    updated_at: String,
}

impl RepoRow {
    fn into_repo(self) -> Repository {
        Repository {
            id: self.id.parse().unwrap_or_default(),
            owner: self.owner_id,
            name: self.name,
            description: self.description,
            visibility: match self.visibility.as_str() {
                "public" => Visibility::Public,
                "internal" => Visibility::Internal,
                _ => Visibility::Private,
            },
            default_branch: self.default_branch,
            created_at: self.created_at.parse().unwrap_or_default(),
            updated_at: self.updated_at.parse().unwrap_or_default(),
        }
    }
}
