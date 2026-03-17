-- Add is_admin flag to users table (idempotent for SQLite).
-- SQLite does not support ALTER TABLE ADD COLUMN IF NOT EXISTS,
-- so we create a temp trigger that does nothing if the column exists.
-- Instead, we just catch the error in the application layer.

-- This is handled programmatically in db.rs init_pool.
-- See the special case for migration 014.
