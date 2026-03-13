-- Phase 8: Federation and mirroring support.
-- Adds mirror tracking to repositories and federation metadata.

ALTER TABLE repositories ADD COLUMN is_mirror BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE repositories ADD COLUMN mirror_url TEXT;
ALTER TABLE repositories ADD COLUMN federation_instance_id TEXT REFERENCES federation_instances(id) ON DELETE SET NULL;
