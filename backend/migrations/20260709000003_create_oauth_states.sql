CREATE TABLE oauth_states (
    state_hash TEXT PRIMARY KEY,
    pkce_verifier TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX idx_oauth_states_expires_at ON oauth_states (expires_at);
