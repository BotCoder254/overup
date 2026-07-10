-- Pipeline Execution hardening pass: runner-reported resource metrics,
-- R2 log archival bookkeeping, and janitor/filter indexes.

-- Runner-reported resource metrics (validated and clamped server-side before
-- storage; camelCase keys matching the API DTOs).
ALTER TABLE pipeline_jobs ADD COLUMN metrics JSONB;

-- Set once the job's full log has been archived to R2
-- (logs/{workspace}/{pipeline}/{job}-{attempt}.log.gz). NULL means the
-- Postgres chunks are still the only copy and must never be pruned.
ALTER TABLE pipeline_jobs ADD COLUMN logs_archived_at TIMESTAMPTZ;

-- Janitor scans: expired uploaded artifacts and stale pending rows.
CREATE INDEX artifacts_expiry_idx ON artifacts (expires_at)
    WHERE status = 'uploaded';
CREATE INDEX artifacts_stale_pending_idx ON artifacts (created_at)
    WHERE status = 'pending';

-- Log-chunk pruning: archived jobs whose hot window has lapsed.
CREATE INDEX pipeline_jobs_archived_idx ON pipeline_jobs (finished_at)
    WHERE logs_archived_at IS NOT NULL;

-- Branch filter on the execution ledger.
CREATE INDEX pipelines_ws_ref_idx ON pipelines (workspace_id, git_ref);

-- Deliberate non-change: pipeline_events.runner_id / actor_user_id stay
-- FK-less. The events table is an append-only history ledger that must
-- outlive runner and user deletion; the ids are advisory display-only
-- context, so referential enforcement buys nothing here.
