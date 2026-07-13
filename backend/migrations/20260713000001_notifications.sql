-- Notification Center: a per-user, actionable projection over the immutable
-- audit ledger. The Activity Feed keeps full history; notifications hold only
-- the events a user should act on, with per-user read/archive state. Rows are
-- written exclusively by the notification projector (audit tail) and the
-- janitor's derived-condition scans — never inside request transactions.

CREATE TABLE notifications (
    id               UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    workspace_id     UUID NOT NULL REFERENCES workspaces (id) ON DELETE CASCADE,
    user_id          UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- Source audit action (or janitor scan kind, e.g. 'secret.stale').
    action           TEXT NOT NULL CHECK (char_length(action) <= 100),
    category         TEXT NOT NULL CHECK (category IN
        ('pipeline', 'runner', 'repository', 'workflow', 'artifact',
         'security', 'environment', 'system')),
    severity         TEXT NOT NULL CHECK (severity IN
        ('info', 'success', 'warning', 'error', 'critical')),
    -- Rendered server-side from static templates + DB name lookups; never
    -- runner/upstream text and never secret values.
    title            TEXT NOT NULL CHECK (char_length(title) <= 300),
    body             TEXT NOT NULL DEFAULT '' CHECK (char_length(body) <= 1000),
    subject_type     TEXT,
    subject_id       UUID,
    -- Client-side route target: {"kind": "...", "...Id": "..."} built from a
    -- server-side allow-list of kinds. Never a URL.
    link             JSONB NOT NULL DEFAULT '{}'::jsonb,
    -- Grouping identity: repeated occurrences merge into the live unread row
    -- (occurrence_count++). NULL = one-shot event, every occurrence its own row.
    dedup_key        TEXT CHECK (char_length(dedup_key) <= 200),
    occurrence_count INT NOT NULL DEFAULT 1 CHECK (occurrence_count >= 1),
    read_at          TIMESTAMPTZ,
    archived_at      TIMESTAMPTZ,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Keyset list per user (history page includes archived; popover excludes it).
CREATE INDEX notifications_user_feed_idx
    ON notifications (user_id, workspace_id, created_at DESC, id DESC);

-- Unread badge count rides this partial index.
CREATE INDEX notifications_unread_idx
    ON notifications (user_id, workspace_id)
    WHERE read_at IS NULL AND archived_at IS NULL;

-- Dedup upsert target: at most ONE live (unread, unarchived) row per
-- (user, dedup_key) — makes concurrent projector batches race-free.
CREATE UNIQUE INDEX notifications_dedup_key
    ON notifications (user_id, dedup_key)
    WHERE read_at IS NULL AND archived_at IS NULL AND dedup_key IS NOT NULL;

-- Janitor scans: auto-archive read rows, purge archived rows past retention.
CREATE INDEX notifications_auto_archive_idx
    ON notifications (read_at)
    WHERE read_at IS NOT NULL AND archived_at IS NULL;
CREATE INDEX notifications_purge_idx
    ON notifications (archived_at)
    WHERE archived_at IS NOT NULL;

-- Per-user delivery preferences; an absent row means all defaults (every
-- category on, min severity 'info', not muted). Enforced at WRITE time in
-- the recipient fan-out, so the read API cannot bypass them. Critical
-- severity always delivers regardless of preferences.
CREATE TABLE notification_preferences (
    user_id             UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    workspace_id        UUID NOT NULL REFERENCES workspaces (id) ON DELETE CASCADE,
    muted_until         TIMESTAMPTZ,
    disabled_categories TEXT[] NOT NULL DEFAULT '{}',
    min_severity        TEXT NOT NULL DEFAULT 'info' CHECK (min_severity IN
        ('info', 'success', 'warning', 'error', 'critical')),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, workspace_id)
);

-- Projector checkpoint over audit_logs (global singleton, advanced in the
-- same transaction as the notification inserts — exactly-once across
-- crashes). Seeded at now(): historical audit rows do not backfill.
CREATE TABLE notification_cursor (
    singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
    last_at   TIMESTAMPTZ NOT NULL,
    last_id   UUID NOT NULL
);

INSERT INTO notification_cursor (last_at, last_id)
VALUES (now(), '00000000-0000-0000-0000-000000000000');
