-- Full-text search index for repository content
CREATE VIRTUAL TABLE IF NOT EXISTS code_search USING fts5(
    repo_id,
    path,
    content,
    tokenize='porter unicode61'
);
