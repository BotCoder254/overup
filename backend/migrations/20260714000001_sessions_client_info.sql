-- Active-sessions UI: capture the client fingerprint at session creation so
-- users can recognize (and revoke) their own sessions. Both columns are
-- nullable — pre-existing sessions simply render as "Unknown device". Values
-- are capped app-side (user agent 256 chars) and the IP is masked before it
-- ever leaves the API.
ALTER TABLE sessions
    ADD COLUMN ip TEXT,
    ADD COLUMN user_agent TEXT;
