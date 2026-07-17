-- Partial index matching `list_active_images` (db/toolchain_images.rs), which
-- runs on every hosted-runner Docker reconnect (30-60 s during an outage) to
-- build the prewarm set. A partial index over just the active rows keeps that
-- lookup index-only regardless of table size.
--
-- Note: today the table is bounded to the toolchain catalog (toolchain_key is
-- the primary key and only catalog keys are ever inserted, so ~10 rows max), so
-- the planner may still seq-scan it — the index is defensive/forward-looking,
-- not a hot-path necessity at current cardinality. Added as its own migration
-- because migrations are immutable once applied (editing the create-table
-- migration would fail sqlx's startup checksum check).
CREATE INDEX IF NOT EXISTS installed_toolchain_images_active_idx
    ON installed_toolchain_images (status)
    WHERE status IN ('installed', 'pending');
