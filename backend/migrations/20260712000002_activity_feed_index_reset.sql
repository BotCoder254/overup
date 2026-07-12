-- Clear any prior audit_logs_ws_created_id_idx before the concurrent build
-- in the next migration: dev databases that applied the original
-- (non-concurrent) version of 20260712000001 already have a valid index
-- under this name, and a manually-run broken attempt could have left an
-- INVALID one — either would make CREATE INDEX CONCURRENTLY IF NOT EXISTS
-- skip silently, so start from a clean slate. Transactional and instant
-- (dropping a missing index is a no-op).
DROP INDEX IF EXISTS audit_logs_ws_created_id_idx;
