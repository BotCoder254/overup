-- Deployment environments (GitHub-style): a named secrets scope per
-- workspace. A workflow job opts in with the YAML `environment:` key; at
-- dispatch the scheduler resolves that name to a row here and injects the
-- environment's secrets with the highest precedence:
--   environment > repository > workspace.
-- No approval gates this phase — environments are metadata + a secrets
-- scope, resolved live by name (a rename changes what future dispatches
-- see; an unknown name skips environment secrets with a plan notice).
CREATE TABLE environments (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    workspace_id  UUID NOT NULL REFERENCES workspaces (id) ON DELETE CASCADE,
    -- Slug-safe charset (no spaces, unlike GitHub) so names embed cleanly
    -- in URLs, badges, and YAML without quoting surprises.
    name          TEXT NOT NULL CHECK (
        name ~ '^[A-Za-z0-9][A-Za-z0-9._-]*$' AND char_length(name) BETWEEN 1 AND 100
    ),
    description   TEXT CHECK (char_length(description) <= 500),
    created_by    UUID REFERENCES users (id) ON DELETE SET NULL,
    updated_by    UUID REFERENCES users (id) ON DELETE SET NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Case-insensitive uniqueness; dispatch lookup matches on lower(name) so
-- it rides this same functional index.
CREATE UNIQUE INDEX environments_ws_name_key
    ON environments (workspace_id, lower(name));
-- Keyset pagination for the catalog (secrets_ws_created_idx shape).
CREATE INDEX environments_ws_created_idx
    ON environments (workspace_id, created_at DESC, id DESC);

-- Third secrets scope. Mutually exclusive with repository scope: an
-- environment is workspace-level here, so a combined repo+env tier would
-- add precedence complexity for no current need.
ALTER TABLE secrets
    ADD COLUMN environment_id UUID REFERENCES environments (id) ON DELETE CASCADE,
    ADD CONSTRAINT secrets_scope_exclusive
        CHECK (repository_id IS NULL OR environment_id IS NULL);

-- The workspace-scope uniqueness predicate must now also exclude
-- environment-scoped rows. Recreated under the SAME NAME — db/secrets.rs
-- classifies unique violations by these exact index names. Safe on
-- existing data: every pre-migration row has environment_id NULL, so the
-- narrower predicate covers exactly the same rows.
DROP INDEX secrets_ws_name_key;
CREATE UNIQUE INDEX secrets_ws_name_key
    ON secrets (workspace_id, name)
    WHERE repository_id IS NULL AND environment_id IS NULL;
CREATE UNIQUE INDEX secrets_env_name_key
    ON secrets (workspace_id, environment_id, name) WHERE environment_id IS NOT NULL;
-- Dispatch-time resolution: all secrets applicable to one environment.
CREATE INDEX secrets_env_resolve_idx
    ON secrets (workspace_id, environment_id) WHERE environment_id IS NOT NULL;

-- RBAC backfill for EXISTING workspaces (the secrets-migration pattern).
-- Listing/detail reuse content.read — environment metadata is not
-- sensitive; only mutations need the new permission.
INSERT INTO role_permissions (role_id, permission)
SELECT r.id, 'environments.manage'
FROM workspace_roles r
WHERE r.key IN ('owner', 'admin')
ON CONFLICT DO NOTHING;
