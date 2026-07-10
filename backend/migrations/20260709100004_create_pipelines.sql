-- Pipeline Execution: runners, pipelines, jobs, the append-only execution
-- event ledger, persisted log chunks, and artifact metadata.
--
-- Status model mirrors GitHub Actions: a coarse `status`
-- (queued|in_progress|completed) that scheduling correctness depends on,
-- plus a `conclusion` set exactly once at completion, plus a fine-grained
-- per-job `stage` that is pure UX telemetry. Every transition is also
-- appended to pipeline_events so the execution history can be reconstructed
-- long after the run finished.

CREATE TABLE runners (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    workspace_id UUID NOT NULL REFERENCES workspaces (id) ON DELETE CASCADE,
    name         TEXT NOT NULL,
    labels       TEXT[] NOT NULL DEFAULT '{}',
    -- SHA-256 hex of the 32-byte OS-RNG registration token; the token itself
    -- is shown exactly once at creation and never stored (sessions pattern).
    token_hash   TEXT NOT NULL UNIQUE,
    status       TEXT NOT NULL DEFAULT 'offline'
        CHECK (status IN ('offline', 'idle', 'busy')),
    version      TEXT,
    last_seen_at TIMESTAMPTZ,
    created_by   UUID REFERENCES users (id) ON DELETE SET NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    revoked_at   TIMESTAMPTZ,
    CONSTRAINT runners_ws_name_key UNIQUE (workspace_id, name)
);

CREATE INDEX runners_ws_idx ON runners (workspace_id);

-- Per-repository run numbering; claimed race-free with an upsert +
-- UPDATE ... RETURNING inside the pipeline creation transaction.
CREATE TABLE pipeline_counters (
    repository_id UUID PRIMARY KEY REFERENCES repositories (id) ON DELETE CASCADE,
    next_number   INTEGER NOT NULL DEFAULT 1
);

CREATE TABLE pipelines (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    workspace_id    UUID NOT NULL REFERENCES workspaces (id) ON DELETE CASCADE,
    repository_id   UUID NOT NULL REFERENCES repositories (id) ON DELETE CASCADE,
    -- Snapshot columns survive workflow deletion/resync; the FK is advisory.
    workflow_id     UUID REFERENCES workflows (id) ON DELETE SET NULL,
    workflow_name   TEXT NOT NULL,
    workflow_path   TEXT NOT NULL,
    number          INTEGER NOT NULL,
    trigger         TEXT NOT NULL CHECK (trigger IN ('push', 'manual')),
    triggered_by    UUID REFERENCES users (id) ON DELETE SET NULL,
    commit_sha      TEXT NOT NULL,
    commit_message  TEXT,
    commit_author   TEXT,
    git_ref         TEXT NOT NULL,
    status          TEXT NOT NULL DEFAULT 'queued'
        CHECK (status IN ('queued', 'in_progress', 'completed')),
    conclusion      TEXT
        CHECK (conclusion IN ('success', 'failure', 'cancelled', 'timed_out', 'partial')),
    timeout_seconds INTEGER NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    started_at      TIMESTAMPTZ,
    finished_at     TIMESTAMPTZ,
    -- A conclusion exists exactly when the pipeline is completed.
    CONSTRAINT pipelines_concl_iff_completed
        CHECK ((status = 'completed') = (conclusion IS NOT NULL)),
    CONSTRAINT pipelines_repo_number_key UNIQUE (repository_id, number)
);

CREATE INDEX pipelines_ws_created_idx ON pipelines (workspace_id, created_at DESC);
CREATE INDEX pipelines_active_idx ON pipelines (workspace_id) WHERE status <> 'completed';

CREATE TABLE pipeline_jobs (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    pipeline_id     UUID NOT NULL REFERENCES pipelines (id) ON DELETE CASCADE,
    job_key         TEXT NOT NULL,
    name            TEXT,
    runs_on         TEXT[] NOT NULL DEFAULT '{}',
    needs           TEXT[] NOT NULL DEFAULT '{}',
    -- Executable snapshot: { image, env, steps: [{name, run, shell}], skippedUses }.
    -- Never contains secret values.
    plan            JSONB NOT NULL,
    status          TEXT NOT NULL DEFAULT 'queued'
        CHECK (status IN ('queued', 'in_progress', 'completed')),
    conclusion      TEXT
        CHECK (conclusion IN ('success', 'failure', 'cancelled', 'timed_out', 'skipped')),
    stage           TEXT NOT NULL DEFAULT 'queued',
    runner_id       UUID REFERENCES runners (id) ON DELETE SET NULL,
    attempt         INTEGER NOT NULL DEFAULT 1,
    exit_code       INTEGER,
    -- Static category strings only (runner_lost, timeout, ...), never
    -- upstream/runner-supplied text.
    error_category  TEXT,
    timeout_seconds INTEGER NOT NULL,
    log_bytes       BIGINT NOT NULL DEFAULT 0,
    position        INTEGER NOT NULL DEFAULT 0,
    queued_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    assigned_at     TIMESTAMPTZ,
    started_at      TIMESTAMPTZ,
    finished_at     TIMESTAMPTZ,
    CONSTRAINT pipeline_jobs_concl_iff_completed
        CHECK ((status = 'completed') = (conclusion IS NOT NULL)),
    CONSTRAINT pipeline_jobs_pipeline_key_key UNIQUE (pipeline_id, job_key)
);

CREATE INDEX pipeline_jobs_pipeline_idx ON pipeline_jobs (pipeline_id);
CREATE INDEX pipeline_jobs_claimable_idx ON pipeline_jobs (queued_at) WHERE status = 'queued';
CREATE INDEX pipeline_jobs_runner_active_idx ON pipeline_jobs (runner_id) WHERE status = 'in_progress';

-- Append-only transition ledger. Payloads carry contextual metadata only —
-- never environment values or secrets.
CREATE TABLE pipeline_events (
    id            BIGSERIAL PRIMARY KEY,
    pipeline_id   UUID NOT NULL REFERENCES pipelines (id) ON DELETE CASCADE,
    job_id        UUID REFERENCES pipeline_jobs (id) ON DELETE CASCADE,
    event_type    TEXT NOT NULL,
    from_state    TEXT,
    to_state      TEXT,
    runner_id     UUID,
    actor_user_id UUID,
    payload       JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX pipeline_events_pipeline_idx ON pipeline_events (pipeline_id, id);

-- Persisted log chunks: masked and truncated server-side BEFORE insert, so
-- nothing sensitive ever reaches disk. seq is runner-monotonic per job.
CREATE TABLE pipeline_log_chunks (
    job_id     UUID NOT NULL REFERENCES pipeline_jobs (id) ON DELETE CASCADE,
    seq        BIGINT NOT NULL,
    stream     TEXT NOT NULL CHECK (stream IN ('stdout', 'stderr', 'system')),
    content    TEXT NOT NULL,
    byte_len   INTEGER NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (job_id, seq)
);

-- Artifact metadata; blobs live in R2 (uploaded via presigned PUT, verified
-- with HeadObject before the row flips to 'uploaded').
CREATE TABLE artifacts (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    workspace_id    UUID NOT NULL REFERENCES workspaces (id) ON DELETE CASCADE,
    pipeline_id     UUID NOT NULL REFERENCES pipelines (id) ON DELETE CASCADE,
    job_id          UUID NOT NULL REFERENCES pipeline_jobs (id) ON DELETE CASCADE,
    name            TEXT NOT NULL,
    r2_key          TEXT NOT NULL UNIQUE,
    size_bytes      BIGINT,
    content_type    TEXT,
    checksum_sha256 TEXT,
    status          TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'uploaded', 'failed', 'expired')),
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at      TIMESTAMPTZ,
    CONSTRAINT artifacts_job_name_key UNIQUE (job_id, name)
);

CREATE INDEX artifacts_pipeline_idx ON artifacts (pipeline_id);
