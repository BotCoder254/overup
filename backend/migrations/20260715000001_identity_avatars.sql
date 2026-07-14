-- Identity avatars: repository owner (refreshed by every sync) and the
-- pipeline actor snapshot (webhook sender / dispatching user, set at
-- creation — pipelines are immutable records, so this is a snapshot, not
-- a join).
ALTER TABLE repositories ADD COLUMN owner_avatar_url text;

ALTER TABLE pipelines
    ADD COLUMN actor_login text,
    ADD COLUMN actor_avatar_url text;

-- Backfill manual/rerun pipelines from their triggering user; push
-- pipelines stay NULL (sender identity was never captured — the UI falls
-- back to commit_author text + monogram). Repositories backfill on their
-- next sync.
UPDATE pipelines p
SET actor_login = u.username,
    actor_avatar_url = u.avatar_url
FROM users u
WHERE p.triggered_by = u.id
  AND p.actor_login IS NULL;
