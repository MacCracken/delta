-- A key must identify exactly one account. Only the key text was unique, so
-- the same key with a different comment could be registered again (by the
-- same or another user), making SSH logins ambiguous. Keep the earliest
-- registration of each key.
DELETE FROM ssh_keys
WHERE rowid NOT IN (SELECT MIN(rowid) FROM ssh_keys GROUP BY fingerprint);
DROP INDEX IF EXISTS idx_ssh_keys_fingerprint;
CREATE UNIQUE INDEX IF NOT EXISTS idx_ssh_keys_fingerprint ON ssh_keys(fingerprint);
