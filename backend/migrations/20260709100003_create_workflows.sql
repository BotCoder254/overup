-- Normalized workflow metadata extracted from .github/workflows. The raw
-- YAML is mirrored (capped at 512 KB before insert) so the editor can render
-- without a GitHub round-trip; blob_sha drives change detection during sync.
CREATE TABLE workflows (
    id                  UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    repository_id       UUID NOT NULL REFERENCES repositories (id) ON DELETE CASCADE,
    path                TEXT NOT NULL,
    name                TEXT NOT NULL,
    blob_sha            TEXT NOT NULL,
    file_size           INTEGER NOT NULL,
    raw_content         TEXT NOT NULL,
    triggers            TEXT[] NOT NULL DEFAULT '{}',
    metadata            JSONB NOT NULL DEFAULT '{}'::jsonb,
    job_count           INTEGER NOT NULL DEFAULT 0,
    validation_status   TEXT NOT NULL DEFAULT 'valid'
        CHECK (validation_status IN ('valid', 'warnings', 'errors')),
    validation_errors   JSONB NOT NULL DEFAULT '[]'::jsonb,
    last_commit_sha     TEXT,
    last_commit_message TEXT,
    last_commit_at      TIMESTAMPTZ,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT workflows_repo_path_key UNIQUE (repository_id, path)
);

CREATE INDEX workflows_repository_idx ON workflows (repository_id);

CREATE TABLE workflow_jobs (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    workflow_id UUID NOT NULL REFERENCES workflows (id) ON DELETE CASCADE,
    job_key     TEXT NOT NULL,
    name        TEXT,
    runs_on     TEXT[] NOT NULL DEFAULT '{}',
    needs       TEXT[] NOT NULL DEFAULT '{}',
    uses        TEXT,
    strategy    JSONB,
    step_count  INTEGER NOT NULL DEFAULT 0,
    position    INTEGER NOT NULL DEFAULT 0,
    CONSTRAINT workflow_jobs_workflow_key_key UNIQUE (workflow_id, job_key)
);

CREATE INDEX workflow_jobs_workflow_idx ON workflow_jobs (workflow_id);
