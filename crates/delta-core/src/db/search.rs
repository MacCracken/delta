//! Code search using SQLite FTS5 full-text search.

use crate::Result;
use serde::Serialize;
use sqlx::SqlitePool;

#[derive(Debug, Clone, Serialize)]
pub struct SearchResult {
    pub repo_id: String,
    pub path: String,
    pub snippet: String,
    pub rank: f64,
}

/// Index a file's content for search.
pub async fn index_file(pool: &SqlitePool, repo_id: &str, path: &str, content: &str) -> Result<()> {
    // Delete existing entry for this file, then insert new one
    sqlx::query("DELETE FROM code_search WHERE repo_id = ? AND path = ?")
        .bind(repo_id)
        .bind(path)
        .execute(pool)
        .await
        .map_err(|e| crate::DeltaError::Storage(e.to_string()))?;

    sqlx::query("INSERT INTO code_search (repo_id, path, content) VALUES (?, ?, ?)")
        .bind(repo_id)
        .bind(path)
        .bind(content)
        .execute(pool)
        .await
        .map_err(|e| crate::DeltaError::Storage(e.to_string()))?;

    Ok(())
}

/// Remove all indexed content for a repository.
pub async fn remove_repo(pool: &SqlitePool, repo_id: &str) -> Result<()> {
    sqlx::query("DELETE FROM code_search WHERE repo_id = ?")
        .bind(repo_id)
        .execute(pool)
        .await
        .map_err(|e| crate::DeltaError::Storage(e.to_string()))?;
    Ok(())
}

/// Search for code matching a query within a specific repository.
pub async fn search_repo(
    pool: &SqlitePool,
    repo_id: &str,
    query: &str,
    limit: u32,
) -> Result<Vec<SearchResult>> {
    // Sanitize the FTS5 query
    let safe_query = sanitize_fts_query(query);

    let rows = sqlx::query_as::<_, SearchRow>(
        "SELECT repo_id, path, snippet(code_search, 2, '<mark>', '</mark>', '...', 40) as snippet, \
         rank FROM code_search WHERE repo_id = ? AND code_search MATCH ? ORDER BY rank LIMIT ?",
    )
    .bind(repo_id)
    .bind(&safe_query)
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(|e| crate::DeltaError::Storage(e.to_string()))?;

    Ok(rows
        .into_iter()
        .map(|r| SearchResult {
            repo_id: r.repo_id,
            path: r.path,
            snippet: r.snippet,
            rank: r.rank,
        })
        .collect())
}

/// Search across all repositories the user has access to.
pub async fn search_global(
    pool: &SqlitePool,
    query: &str,
    repo_ids: &[String],
    limit: u32,
) -> Result<Vec<SearchResult>> {
    if repo_ids.is_empty() {
        return Ok(Vec::new());
    }

    let safe_query = sanitize_fts_query(query);

    // Build a query with the right number of placeholders
    let placeholders: Vec<&str> = repo_ids.iter().map(|_| "?").collect();
    let sql = format!(
        "SELECT repo_id, path, snippet(code_search, 2, '<mark>', '</mark>', '...', 40) as snippet, \
         rank FROM code_search WHERE repo_id IN ({}) AND code_search MATCH ? ORDER BY rank LIMIT ?",
        placeholders.join(","),
    );

    let mut query_builder = sqlx::query_as::<_, SearchRow>(&sql);
    for id in repo_ids {
        query_builder = query_builder.bind(id);
    }
    query_builder = query_builder.bind(&safe_query);
    query_builder = query_builder.bind(limit);

    let rows = query_builder
        .fetch_all(pool)
        .await
        .map_err(|e| crate::DeltaError::Storage(e.to_string()))?;

    Ok(rows
        .into_iter()
        .map(|r| SearchResult {
            repo_id: r.repo_id,
            path: r.path,
            snippet: r.snippet,
            rank: r.rank,
        })
        .collect())
}

/// Sanitize an FTS5 query string to prevent injection.
fn sanitize_fts_query(query: &str) -> String {
    // Wrap each word in quotes to treat them as literal terms
    query
        .split_whitespace()
        .map(|word| {
            let clean: String = word
                .chars()
                .filter(|c| !matches!(c, '"' | '\'' | '*' | '(' | ')' | '{' | '}' | ':'))
                .collect();
            format!("\"{}\"", clean)
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[derive(sqlx::FromRow)]
struct SearchRow {
    repo_id: String,
    path: String,
    snippet: String,
    rank: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_fts_query() {
        assert_eq!(sanitize_fts_query("hello world"), "\"hello\" \"world\"");
        assert_eq!(sanitize_fts_query("fn main()"), "\"fn\" \"main\"");
        assert_eq!(sanitize_fts_query("test\"injection"), "\"testinjection\"");
    }
}
