-- no-transaction
-- (CREATE INDEX CONCURRENTLY cannot run inside a transaction block, so this
-- migration opts out of sqlx's per-migration transaction.)
--
-- Activity Feed: workspace-wide reader over the immutable audit_logs ledger.
--
-- 1. Keyset-pagination index. The feed pages on the (created_at, id) tuple
--    (the secrets_ws_created_idx shape); the original
--    audit_logs_workspace_created_idx stays in place to keep this additive.
--    Built CONCURRENTLY so a production ledger keeps accepting audit writes
--    (every workspace mutation records one) during the build. A failed
--    concurrent build leaves an INVALID index behind, so the DROP makes the
--    retry after such a failure idempotent.
DROP INDEX IF EXISTS audit_logs_ws_created_id_idx;
CREATE INDEX CONCURRENTLY audit_logs_ws_created_id_idx
    ON audit_logs (workspace_id, created_at DESC, id DESC);

-- 2. RBAC backfill. audit.read is already seeded into owner/admin roles by
--    the Rust provisioning arrays, but those only run at workspace creation
--    — existing workspaces need the rows, and the feed is readable by every
--    member, so member roles get it too (matching the updated
--    MEMBER_PERMISSIONS array in db/workspaces.rs).
INSERT INTO role_permissions (role_id, permission)
SELECT r.id, 'audit.read'
FROM workspace_roles r
WHERE r.key IN ('owner', 'admin', 'member')
ON CONFLICT DO NOTHING;
