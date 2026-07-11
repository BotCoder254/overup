-- Rotation visibility: when the VALUE was last set (created or replaced).
-- updated_at also moves on metadata edits (description), so it cannot serve
-- as the rotation clock. Existing rows backfill from updated_at as the best
-- available proxy.
ALTER TABLE secrets
    ADD COLUMN value_set_at TIMESTAMPTZ NOT NULL DEFAULT now();
UPDATE secrets SET value_set_at = updated_at;
