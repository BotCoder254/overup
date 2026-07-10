-- Connected repositories. GitHub remains the source of truth; these rows are
-- the normalized mirror that the UI and future pipeline execution key off.
CREATE TABLE repositories (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    workspace_id    UUID NOT NULL REFERENCES workspaces (id) ON DELETE CASCADE,
    installation_id UUID NOT NULL REFERENCES github_installations (id) ON DELETE CASCADE,
    github_repo_id  BIGINT NOT NULL,
    owner           TEXT NOT NULL,
    name            TEXT NOT NULL,
    full_name       TEXT NOT NULL,
    private         BOOLEAN NOT NULL DEFAULT false,
    default_branch  TEXT NOT NULL DEFAULT 'main',
    language        TEXT,
    description     TEXT,
    sync_status     TEXT NOT NULL DEFAULT 'pending'
        CHECK (sync_status IN ('pending', 'syncing', 'synced', 'failed')),
    -- Static category strings only — never raw upstream error bodies.
    sync_error      TEXT,
    last_synced_at  TIMESTAMPTZ,
    imported_by     UUID REFERENCES users (id) ON DELETE SET NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- Named: the import handler classifies duplicate-import races into 409.
    CONSTRAINT repositories_workspace_repo_key UNIQUE (workspace_id, github_repo_id)
);

CREATE INDEX repositories_workspace_idx ON repositories (workspace_id);
CREATE INDEX repositories_github_repo_idx ON repositories (github_repo_id);

CREATE TABLE repo_branches (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    repository_id UUID NOT NULL REFERENCES repositories (id) ON DELETE CASCADE,
    name          TEXT NOT NULL,
    commit_sha    TEXT NOT NULL,
    is_default    BOOLEAN NOT NULL DEFAULT false,
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT repo_branches_repo_name_key UNIQUE (repository_id, name)
);

CREATE TABLE repo_sync_runs (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    repository_id UUID NOT NULL REFERENCES repositories (id) ON DELETE CASCADE,
    trigger       TEXT NOT NULL CHECK (trigger IN ('import', 'manual', 'webhook')),
    status        TEXT NOT NULL DEFAULT 'running'
        CHECK (status IN ('running', 'success', 'failed')),
    error         TEXT,
    stats         JSONB NOT NULL DEFAULT '{}'::jsonb,
    started_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    finished_at   TIMESTAMPTZ
);

CREATE INDEX repo_sync_runs_repo_started_idx
    ON repo_sync_runs (repository_id, started_at DESC);

-- Webhook idempotency + observability. X-GitHub-Delivery is unique per
-- delivery; a replayed delivery hits the primary key and becomes a no-op.
CREATE TABLE webhook_deliveries (
    delivery_id     TEXT PRIMARY KEY,
    event           TEXT NOT NULL,
    action          TEXT,
    installation_id BIGINT,
    status          TEXT NOT NULL DEFAULT 'processed'
        CHECK (status IN ('processed', 'ignored', 'failed')),
    received_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);
