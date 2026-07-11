-- Workspace-wide artifact catalog: keyset pagination on
-- (workspace_id, created_at DESC, id DESC).
CREATE INDEX artifacts_ws_created_idx ON artifacts (workspace_id, created_at DESC, id DESC);
