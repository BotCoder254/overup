-- Workspace logo lives in R2 (same store as artifacts/log archives); the row
-- only holds the server-generated object key. NULL = no logo.
ALTER TABLE workspaces
    ADD COLUMN logo_key TEXT;
