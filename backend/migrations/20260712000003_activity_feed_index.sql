-- no-transaction
-- Keyset-pagination index for the Activity Feed. The feed pages on the
-- (created_at, id) tuple (the secrets_ws_created_idx shape); the original
-- audit_logs_workspace_created_idx stays in place to keep this additive.
--
-- Built CONCURRENTLY so a production ledger keeps accepting audit writes
-- (every workspace mutation records one) during the build — which is why
-- this migration opts out of sqlx's transaction AND contains exactly one
-- statement: PostgreSQL runs a multi-statement simple-query batch inside
-- an implicit transaction, where CREATE INDEX CONCURRENTLY is rejected.
--
-- IF NOT EXISTS keeps a boot-time retry from hard-blocking startup. The
-- one residual edge: a concurrent build that crashes mid-flight leaves an
-- INVALID index this clause would then skip — remediation is
-- `DROP INDEX audit_logs_ws_created_id_idx;` and a restart (the previous
-- migration's reset pattern).
CREATE INDEX CONCURRENTLY IF NOT EXISTS audit_logs_ws_created_id_idx
    ON audit_logs (workspace_id, created_at DESC, id DESC);
