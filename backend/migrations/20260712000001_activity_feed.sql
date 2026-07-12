-- Activity Feed: workspace-wide reader over the immutable audit_logs ledger.
--
-- RBAC backfill. audit.read is already seeded into owner/admin roles by
-- the Rust provisioning arrays, but those only run at workspace creation
-- — existing workspaces need the rows, and the feed is readable by every
-- member, so member roles get it too (matching the updated
-- MEMBER_PERMISSIONS array in db/workspaces.rs).
--
-- The keyset index lives in the two follow-up migrations: a sqlx
-- `-- no-transaction` migration must contain exactly ONE statement
-- (PostgreSQL wraps a multi-statement simple-query batch in an implicit
-- transaction, which CREATE INDEX CONCURRENTLY rejects), so the index
-- build cannot share a file with this INSERT.
INSERT INTO role_permissions (role_id, permission)
SELECT r.id, 'audit.read'
FROM workspace_roles r
WHERE r.key IN ('owner', 'admin', 'member')
ON CONFLICT DO NOTHING;
