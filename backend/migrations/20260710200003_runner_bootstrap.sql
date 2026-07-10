-- Guided registration: the wizard mints a short-lived bootstrap token (shown
-- once, like today's permanent token) instead of a permanent credential.
-- The runner exchanges it for a permanent token on its first successful
-- connection, at which point bootstrap_token_hash is cleared. A runner row
-- can therefore exist in a "pending registration" state with no permanent
-- token yet — token_hash must allow NULL (UNIQUE still allows many NULLs).
ALTER TABLE runners ALTER COLUMN token_hash DROP NOT NULL;
ALTER TABLE runners
    ADD COLUMN bootstrap_token_hash TEXT UNIQUE,
    ADD COLUMN bootstrap_expires_at TIMESTAMPTZ;
