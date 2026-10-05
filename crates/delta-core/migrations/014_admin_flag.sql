-- Site admin flag.
ALTER TABLE users ADD COLUMN is_admin BOOLEAN NOT NULL DEFAULT FALSE;

-- Existing installs: promote the earliest registered user when nobody is an
-- admin yet. Fresh installs promote the first user at registration instead
-- (see `db::user::ensure_bootstrap_admin`).
UPDATE users SET is_admin = TRUE
WHERE id = (SELECT id FROM users ORDER BY created_at ASC LIMIT 1)
  AND NOT EXISTS (SELECT 1 FROM users WHERE is_admin = TRUE);
