-- Async webhook processing + per-repository event timeline + PR/tag triggers.
--
-- 1. webhook_deliveries grows from a pure idempotency ledger into a durable
--    work queue: the HTTP handler persists a normalized (server-built, capped)
--    payload and acks GitHub immediately; a background worker claims rows and
--    performs sync/trigger-evaluation/pipeline creation asynchronously.
-- 2. repository_events is the immutable per-repo timeline the UI renders:
--    what arrived, what it caused (pipelines/sync), or why it was ignored —
--    outcome/ignored_reason hold STATIC category strings only, never upstream
--    text.
-- 3. pipelines learn the pull_request/tag trigger vocabulary, the PR number,
--    and the GitHub check-run linkage for Checks API reporting.

-- 1. Durable webhook queue -------------------------------------------------

ALTER TABLE webhook_deliveries
    ADD COLUMN payload         JSONB,
    ADD COLUMN github_repo_id  BIGINT,
    ADD COLUMN retry_count     INTEGER NOT NULL DEFAULT 0,
    ADD COLUMN last_attempt_at TIMESTAMPTZ,
    ADD COLUMN processed_at    TIMESTAMPTZ;

ALTER TABLE webhook_deliveries
    DROP CONSTRAINT webhook_deliveries_status_check;
ALTER TABLE webhook_deliveries
    ADD CONSTRAINT webhook_deliveries_status_check
        CHECK (status IN ('pending', 'processing', 'processed', 'ignored', 'failed'));

-- Worker drain scans only live rows, oldest first (per-repo ordering).
CREATE INDEX webhook_deliveries_pending_idx
    ON webhook_deliveries (received_at)
    WHERE status IN ('pending', 'processing');

-- Per-repo webhook health (pending counts) without a workspace join.
CREATE INDEX webhook_deliveries_repo_idx
    ON webhook_deliveries (github_repo_id, received_at DESC)
    WHERE github_repo_id IS NOT NULL;

-- 2. Immutable per-repository event timeline --------------------------------

CREATE TABLE repository_events (
    id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    repository_id    UUID NOT NULL REFERENCES repositories (id) ON DELETE CASCADE,
    delivery_id      TEXT NOT NULL,
    event            TEXT NOT NULL,
    action           TEXT,
    git_ref          TEXT,
    head_sha         TEXT,
    actor_login      TEXT,
    actor_avatar_url TEXT,
    -- Static categories only, enforced by consts in services/webhook_processor.rs.
    outcome          TEXT NOT NULL CHECK (outcome IN
        ('pipelines_created', 'sync_scheduled', 'pipelines_and_sync', 'ignored', 'failed')),
    ignored_reason   TEXT,
    pipeline_ids     UUID[] NOT NULL DEFAULT '{}',
    sync_run_id      UUID,
    -- Server-built, capped summary (skipped workflows, PR number, merged flag).
    summary          JSONB NOT NULL DEFAULT '{}'::jsonb,
    received_at      TIMESTAMPTZ NOT NULL,
    processed_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX repository_events_repo_idx
    ON repository_events (repository_id, processed_at DESC, id DESC);

-- 3. Pipelines: PR/tag triggers + Checks API linkage -------------------------

ALTER TABLE pipelines
    DROP CONSTRAINT pipelines_trigger_check;
ALTER TABLE pipelines
    ADD CONSTRAINT pipelines_trigger_check
        CHECK (trigger IN ('push', 'manual', 'pull_request', 'tag'));

ALTER TABLE pipelines
    ADD COLUMN pr_number    INTEGER,
    ADD COLUMN check_run_id BIGINT;
