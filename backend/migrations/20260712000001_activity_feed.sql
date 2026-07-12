-- Activity Feed: workspace-wide reader over the immutable audit_logs ledger.
--
-- 1. Keyset-pagination index. The feed pages on the (created_at, id) tuple
--    (the secrets_ws_created_idx shape); the original
--    audit_logs_workspace_created_idx stays in place to keep this additive.
CREATE INDEX audit_logs_ws_created_id_idx
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
