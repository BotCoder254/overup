-- Lifecycle controls: an operator can disable a runner (stop scheduling new
-- work onto it, equivalent to a temporary pause) or drain it (finish the
-- current job, then go offline instead of idle, without accepting new
-- work in between). Disable widens the status enum because the scheduler's
-- hot-path query already excludes anything that isn't 'idle' — no query
-- changes needed there. Draining is kept as a separate timestamp because a
-- draining runner must still read as 'busy' while finishing its job.
ALTER TABLE runners DROP CONSTRAINT runners_status_check;
ALTER TABLE runners ADD CONSTRAINT runners_status_check
    CHECK (status IN ('offline', 'idle', 'busy', 'disabled'));
ALTER TABLE runners ADD COLUMN draining_at TIMESTAMPTZ;
