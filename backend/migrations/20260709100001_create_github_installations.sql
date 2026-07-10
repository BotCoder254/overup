-- GitHub App installations linked to a workspace. The installation is the
-- credential boundary: every repository operation authenticates with a
-- short-lived installation access token minted from the app JWT — user
-- OAuth tokens are never stored or reused.
CREATE TABLE github_installations (
    id                 UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    workspace_id       UUID NOT NULL REFERENCES workspaces (id) ON DELETE CASCADE,
    installation_id    BIGINT NOT NULL,
    account_login      TEXT NOT NULL,
    account_type       TEXT NOT NULL CHECK (account_type IN ('User', 'Organization')),
    account_avatar_url TEXT,
    linked_by          UUID REFERENCES users (id) ON DELETE SET NULL,
    suspended_at       TIMESTAMPTZ,
    created_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- Named: the setup handler classifies "already linked elsewhere" races.
    CONSTRAINT github_installations_installation_id_key UNIQUE (installation_id)
);

CREATE INDEX github_installations_workspace_idx ON github_installations (workspace_id);

-- Installations observed only via the `installation.created` webhook.
-- Org installs land here until an authorized workspace member claims them
-- through the GitHub App setup redirect; sender_login proves who installed.
CREATE TABLE github_installation_events (
    installation_id BIGINT PRIMARY KEY,
    account_login   TEXT NOT NULL,
    sender_login    TEXT NOT NULL,
    received_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);
