use crate::{DeltaError, Result};
use crate::models::branch_protection::BranchProtection;
use sqlx::SqlitePool;
use uuid::Uuid;

/// Parameters for creating a branch protection rule.
pub struct CreateParams<'a> {
    pub repo_id: &'a str,
    pub pattern: &'a str,
    pub require_pr: bool,
    pub required_approvals: u32,
    pub require_status_checks: bool,
    pub prevent_force_push: bool,
    pub prevent_deletion: bool,
}

/// Create a branch protection rule.
pub async fn create(pool: &SqlitePool, params: CreateParams<'_>) -> Result<BranchProtection> {
    let id = Uuid::new_v4();

    sqlx::query(
        "INSERT INTO branch_protections (id, repo_id, pattern, require_pr, required_approvals, require_status_checks, prevent_force_push, prevent_deletion)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(id.to_string())
    .bind(params.repo_id)
    .bind(params.pattern)
    .bind(params.require_pr)
    .bind(params.required_approvals as i64)
    .bind(params.require_status_checks)
    .bind(params.prevent_force_push)
    .bind(params.prevent_deletion)
    .execute(pool)
    .await
    .map_err(|e| {
        if e.to_string().contains("UNIQUE") {
            DeltaError::Conflict(format!("protection for pattern '{}' already exists", params.pattern))
        } else {
            DeltaError::Storage(e.to_string())
        }
    })?;

    Ok(BranchProtection {
        id,
        repo_id: params.repo_id.parse().unwrap_or_default(),
        pattern: params.pattern.to_string(),
        require_pr: params.require_pr,
        required_approvals: params.required_approvals,
        require_status_checks: params.require_status_checks,
        prevent_force_push: params.prevent_force_push,
        prevent_deletion: params.prevent_deletion,
    })
}

/// Get all branch protections for a repository.
pub async fn list_for_repo(pool: &SqlitePool, repo_id: &str) -> Result<Vec<BranchProtection>> {
    let rows = sqlx::query_as::<_, ProtectionRow>(
        "SELECT * FROM branch_protections WHERE repo_id = ?",
    )
    .bind(repo_id)
    .fetch_all(pool)
    .await
    .map_err(|e| DeltaError::Storage(e.to_string()))?;

    Ok(rows.into_iter().map(|r| r.into_protection()).collect())
}

/// Find the protection rule that matches a given branch name.
pub async fn find_matching(
    pool: &SqlitePool,
    repo_id: &str,
    branch: &str,
) -> Result<Option<BranchProtection>> {
    let protections = list_for_repo(pool, repo_id).await?;
    Ok(protections.into_iter().find(|p| p.matches(branch)))
}

/// Delete a branch protection rule.
pub async fn delete(pool: &SqlitePool, protection_id: &str) -> Result<()> {
    let result = sqlx::query("DELETE FROM branch_protections WHERE id = ?")
        .bind(protection_id)
        .execute(pool)
        .await
        .map_err(|e| DeltaError::Storage(e.to_string()))?;

    if result.rows_affected() == 0 {
        return Err(DeltaError::RepoNotFound("protection rule not found".into()));
    }
    Ok(())
}

#[derive(sqlx::FromRow)]
struct ProtectionRow {
    id: String,
    repo_id: String,
    pattern: String,
    require_pr: bool,
    required_approvals: i64,
    require_status_checks: bool,
    prevent_force_push: bool,
    prevent_deletion: bool,
    #[allow(dead_code)]
    created_at: String,
}

impl ProtectionRow {
    fn into_protection(self) -> BranchProtection {
        BranchProtection {
            id: self.id.parse().unwrap_or_default(),
            repo_id: self.repo_id.parse().unwrap_or_default(),
            pattern: self.pattern,
            require_pr: self.require_pr,
            required_approvals: self.required_approvals as u32,
            require_status_checks: self.require_status_checks,
            prevent_force_push: self.prevent_force_push,
            prevent_deletion: self.prevent_deletion,
        }
    }
}
