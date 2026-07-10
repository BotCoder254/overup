-- Ambient host telemetry reported by runners on every heartbeat. Last-value
-- only (no time-series table): unbounded history would need a retention
-- policy this phase deliberately defers.
ALTER TABLE runners
    ADD COLUMN last_health JSONB,
    ADD COLUMN last_health_at TIMESTAMPTZ;
