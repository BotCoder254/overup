-- Secrets Management: write-only, envelope-encrypted credentials owned by
-- the control plane. Only ciphertext, nonces, the wrapped per-secret DEK,
-- and metadata are stored — plaintext values NEVER appear in this table,
-- in audit_logs metadata, or in pipeline_jobs.plan. Values are decrypted
-- exclusively at scheduler dispatch, injected into the signed job payload,
-- and registered as log masks before the payload leaves the process.
CREATE TABLE secrets (
    -- Generated in Rust (not by the DB default) because the id doubles as
    -- the AEAD associated data: a ciphertext moved onto another row fails
    -- authentication.
    id            UUID PRIMARY KEY,
    workspace_id  UUID NOT NULL REFERENCES workspaces (id) ON DELETE CASCADE,
    -- NULL = workspace-scoped (visible to every repository in the
    -- workspace); set = repository-scoped, overriding a workspace secret
    -- of the same name for that repository's pipelines.
    repository_id UUID REFERENCES repositories (id) ON DELETE CASCADE,
    name          TEXT NOT NULL CHECK (
        name ~ '^[A-Z_][A-Z0-9_]*$' AND char_length(name) BETWEEN 1 AND 200
    ),
    description   TEXT CHECK (char_length(description) <= 500),
    ciphertext    BYTEA NOT NULL,   -- AES-256-GCM(value, DEK)
    nonce         BYTEA NOT NULL CHECK (octet_length(nonce) = 12),
    wrapped_dek   BYTEA NOT NULL,   -- AES-256-GCM(DEK, SECRETS_MASTER_KEY)
    dek_nonce     BYTEA NOT NULL CHECK (octet_length(dek_nonce) = 12),
    -- Master-key generation used to wrap the DEK; reserved for future
    -- key rotation (only version 1 exists today).
    key_version   INTEGER NOT NULL DEFAULT 1,
    created_by    UUID REFERENCES users (id) ON DELETE SET NULL,
    updated_by    UUID REFERENCES users (id) ON DELETE SET NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_used_at  TIMESTAMPTZ,
    usage_count   BIGINT NOT NULL DEFAULT 0
);

-- Duplicate detection per scope via named partial unique indexes; the Rust
-- db layer classifies unique violations by these exact names (the
-- workspaces_slug_key pattern).
CREATE UNIQUE INDEX secrets_ws_name_key
    ON secrets (workspace_id, name) WHERE repository_id IS NULL;
CREATE UNIQUE INDEX secrets_repo_name_key
    ON secrets (workspace_id, repository_id, name) WHERE repository_id IS NOT NULL;

-- Keyset pagination for the catalog (artifacts_ws_created_idx shape).
CREATE INDEX secrets_ws_created_idx
    ON secrets (workspace_id, created_at DESC, id DESC);
-- Dispatch-time resolution: all secrets applicable to one repository.
CREATE INDEX secrets_resolve_idx ON secrets (workspace_id, repository_id);

-- RBAC backfill for EXISTING workspaces. New workspaces receive these from
-- the provisioning arrays in db/workspaces.rs, which only run at creation
-- time — without this INSERT every pre-existing workspace would 403 on the
-- new endpoints even for owners.
INSERT INTO role_permissions (role_id, permission)
SELECT r.id, p.perm
FROM workspace_roles r
JOIN (VALUES ('secrets.read'), ('secrets.manage')) AS p(perm) ON TRUE
WHERE (p.perm = 'secrets.read'   AND r.key IN ('owner', 'admin', 'member'))
   OR (p.perm = 'secrets.manage' AND r.key IN ('owner', 'admin'))
ON CONFLICT DO NOTHING;
