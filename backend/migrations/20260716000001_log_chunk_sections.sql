-- Per-chunk section attribution for the Live Logs workspace.
--
-- Nullable and additive: chunks from pre-section runners (or non-step
-- output like the overflow marker) keep NULL and render outside any
-- collapsible section. `step_index` is a 0-based index into the job's
-- signed plan steps, bounds-checked server-side before insert; `phase`
-- is allow-listed against protocol::LOG_PHASES both in code and here.
ALTER TABLE pipeline_log_chunks
    ADD COLUMN step_index SMALLINT,
    ADD COLUMN phase TEXT CHECK (phase IS NULL OR phase IN
        ('checkout', 'image_pull', 'container', 'steps', 'artifacts', 'cleanup'));
