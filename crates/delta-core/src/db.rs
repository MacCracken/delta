pub mod ark_package;
pub mod artifact;
pub mod audit;
pub mod branch_protection;
pub mod collaborator;
pub mod download_stats;
pub mod encryption;
pub mod federation;
pub mod lfs;
pub mod oci;
pub mod pipeline;
pub mod pull_request;
pub mod release;
pub mod repo;
pub mod retention;
pub mod runner;
pub mod search;
pub mod secret;
pub mod signing;
pub mod ssh_key;
pub mod status_check;
pub mod user;
pub mod webhook;
pub mod workspace;

use crate::Result;
use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteConnection};
use std::str::FromStr;

/// Embedded schema migrations, applied in order. Each one runs exactly once;
/// applied versions are recorded in the `schema_migrations` table.
const MIGRATIONS: &[(i64, &str)] = &[
    (1, include_str!("../migrations/001_initial.sql")),
    (2, include_str!("../migrations/002_git_protocol.sql")),
    (3, include_str!("../migrations/003_pull_requests.sql")),
    (4, include_str!("../migrations/004_cicd.sql")),
    (5, include_str!("../migrations/005_registry.sql")),
    (6, include_str!("../migrations/006_collaborators.sql")),
    (7, include_str!("../migrations/007_forks_and_templates.sql")),
    (8, include_str!("../migrations/008_lfs.sql")),
    (9, include_str!("../migrations/009_cascade_fixes.sql")),
    (10, include_str!("../migrations/010_search.sql")),
    (11, include_str!("../migrations/011_federation.sql")),
    (12, include_str!("../migrations/012_encryption.sql")),
    (13, include_str!("../migrations/013_workspaces.sql")),
    (14, include_str!("../migrations/014_admin_flag.sql")),
    (15, include_str!("../migrations/015_runners.sql")),
    (16, include_str!("../migrations/016_review_commit.sql")),
    (17, include_str!("../migrations/017_blob_refs.sql")),
    (
        18,
        include_str!("../migrations/018_ssh_key_fingerprint.sql"),
    ),
];

/// Initialize the database connection pool and run migrations.
pub async fn init_pool(db_url: &str) -> Result<SqlitePool> {
    init_pool_sized(db_url, 10).await
}

/// Initialize the database connection pool with a configurable pool size and run migrations.
pub async fn init_pool_sized(db_url: &str, max_connections: u32) -> Result<SqlitePool> {
    // Only SQLite is supported. sqlx would treat any other URL as a file
    // name and silently create a local database (e.g. for a postgres:// URL).
    if !db_url.starts_with("sqlite:") {
        return Err(crate::DeltaError::Storage(format!(
            "unsupported database URL '{}': only sqlite: URLs are supported",
            db_url.split(['@', '?']).next().unwrap_or("")
        )));
    }
    let url = db_url.strip_prefix("sqlite://").unwrap_or(db_url);

    // Ensure parent directory exists
    if let Some(parent) = std::path::Path::new(url).parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }

    let options = SqliteConnectOptions::from_str(db_url)
        .map_err(|e| crate::DeltaError::Storage(e.to_string()))?
        .create_if_missing(true);

    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(max_connections)
        .connect_with(options)
        .await
        .map_err(|e| crate::DeltaError::Storage(e.to_string()))?;

    run_migrations(&pool).await?;

    tracing::info!("database initialized (pool_size={})", max_connections);
    Ok(pool)
}

/// Apply every migration that has not been recorded in `schema_migrations`.
///
/// Each migration runs in its own transaction together with its bookkeeping
/// row, so a failed migration leaves no partial state and is retried on the
/// next start.
pub async fn run_migrations(pool: &SqlitePool) -> Result<()> {
    let storage = |e: sqlx::Error| crate::DeltaError::Storage(e.to_string());

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
            version INTEGER PRIMARY KEY NOT NULL,
            applied_at TEXT NOT NULL DEFAULT (datetime('now'))
        )",
    )
    .execute(pool)
    .await
    .map_err(storage)?;

    for &(version, sql) in MIGRATIONS {
        let applied =
            sqlx::query_scalar::<_, i64>("SELECT version FROM schema_migrations WHERE version = ?")
                .bind(version)
                .fetch_optional(pool)
                .await
                .map_err(storage)?;
        if applied.is_some() {
            continue;
        }

        let mut tx = pool.begin().await.map_err(storage)?;
        let sql = skip_existing_columns(&mut tx, sql).await?;
        sqlx::raw_sql(sqlx::AssertSqlSafe(sql))
            .execute(&mut *tx)
            .await
            .map_err(|e| crate::DeltaError::Storage(format!("migration {version} failed: {e}")))?;
        sqlx::query("INSERT INTO schema_migrations (version) VALUES (?)")
            .bind(version)
            .execute(&mut *tx)
            .await
            .map_err(storage)?;
        tx.commit().await.map_err(storage)?;
        tracing::info!(version, "applied database migration");
    }
    Ok(())
}

/// Drop `ALTER TABLE <t> ADD COLUMN <c>` statements whose column already exists.
///
/// Databases created before migrations were tracked ran every migration on
/// each start, so some columns may already be present; SQLite has no
/// `ADD COLUMN IF NOT EXISTS`. All other statements in the migrations are
/// idempotent (`IF NOT EXISTS`). Such ALTER statements must fit on one line.
async fn skip_existing_columns(conn: &mut SqliteConnection, sql: &str) -> Result<String> {
    let mut out = String::with_capacity(sql.len());
    for line in sql.lines() {
        if let Some((table, column)) = parse_add_column(line) {
            let exists = sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM pragma_table_info(?) WHERE name = ?",
            )
            .bind(table)
            .bind(column)
            .fetch_one(&mut *conn)
            .await
            .map_err(|e| crate::DeltaError::Storage(e.to_string()))?;
            if exists > 0 {
                continue;
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    Ok(out)
}

/// Parse `ALTER TABLE <table> ADD [COLUMN] <column> ...` into (table, column).
fn parse_add_column(line: &str) -> Option<(&str, &str)> {
    let mut words = line.split_whitespace();
    let mut expect = |kw: &str| words.next().is_some_and(|w| w.eq_ignore_ascii_case(kw));
    if !(expect("ALTER") && expect("TABLE")) {
        return None;
    }
    let table = words.next()?;
    if !words.next()?.eq_ignore_ascii_case("ADD") {
        return None;
    }
    let mut column = words.next()?;
    if column.eq_ignore_ascii_case("COLUMN") {
        column = words.next()?;
    }
    Some((table, column))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_add_column() {
        assert_eq!(
            parse_add_column("ALTER TABLE repositories ADD COLUMN forked_from TEXT;"),
            Some(("repositories", "forked_from"))
        );
        assert_eq!(
            parse_add_column("  alter table users add is_admin BOOLEAN"),
            Some(("users", "is_admin"))
        );
        assert_eq!(parse_add_column("CREATE TABLE x (id TEXT);"), None);
        assert_eq!(parse_add_column("-- ALTER TABLE x ADD COLUMN y"), None);
        assert_eq!(parse_add_column("ALTER TABLE x RENAME TO y;"), None);
    }
}
