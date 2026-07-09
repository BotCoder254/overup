-- Workspaces are the root ownership and security boundary. Every future
-- resource (repositories, runners, pipelines, secrets, ...) hangs off one.
CREATE TABLE workspaces (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name        TEXT NOT NULL,
    slug        TEXT NOT NULL,
    description TEXT,
    created_by  UUID NOT NULL REFERENCES users (id),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- Named constraint: the Rust provisioning code classifies unique
    -- violations by constraint name to drive slug-collision retries.
    CONSTRAINT workspaces_slug_key UNIQUE (slug),
    CONSTRAINT workspaces_slug_format CHECK (
        slug ~ '^[a-z0-9]+(-[a-z0-9]+)*$' AND char_length(slug) <= 50
    )
);

CREATE TABLE workspace_roles (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    workspace_id UUID NOT NULL REFERENCES workspaces (id) ON DELETE CASCADE,
    key          TEXT NOT NULL,
    name         TEXT NOT NULL,
    is_system    BOOLEAN NOT NULL DEFAULT true,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT workspace_roles_workspace_key UNIQUE (workspace_id, key)
);

CREATE TABLE role_permissions (
    role_id    UUID NOT NULL REFERENCES workspace_roles (id) ON DELETE CASCADE,
    permission TEXT NOT NULL,
    PRIMARY KEY (role_id, permission)
);

CREATE TABLE workspace_members (
    workspace_id UUID NOT NULL REFERENCES workspaces (id) ON DELETE CASCADE,
    user_id      UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    role_id      UUID NOT NULL REFERENCES workspace_roles (id),
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (workspace_id, user_id),
    -- One workspace per user for now; the authoritative guard against
    -- concurrent double-provisioning. Drop when multi-membership lands.
    CONSTRAINT workspace_members_user_id_key UNIQUE (user_id)
);

CREATE TABLE workspace_settings (
    workspace_id UUID PRIMARY KEY REFERENCES workspaces (id) ON DELETE CASCADE,
    settings     JSONB NOT NULL DEFAULT '{}'::jsonb,
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE audit_logs (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    workspace_id  UUID REFERENCES workspaces (id) ON DELETE SET NULL,
    actor_user_id UUID REFERENCES users (id) ON DELETE SET NULL,
    action        TEXT NOT NULL,
    subject_type  TEXT NOT NULL,
    subject_id    UUID,
    metadata      JSONB NOT NULL DEFAULT '{}'::jsonb,
    request_id    TEXT,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX audit_logs_workspace_created_idx
    ON audit_logs (workspace_id, created_at DESC);
