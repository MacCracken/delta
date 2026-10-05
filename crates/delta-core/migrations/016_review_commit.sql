-- Record which commit a review applies to, so approvals of an older head
-- don't count once new commits are pushed.
ALTER TABLE pr_reviews ADD COLUMN commit_sha TEXT;
