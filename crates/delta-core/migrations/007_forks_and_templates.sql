-- Phase 2: Repository forking + Phase 4: Reusable workflow templates
-- Add fork tracking to repositories
ALTER TABLE repositories ADD COLUMN forked_from TEXT REFERENCES repositories(id) ON DELETE SET NULL;
