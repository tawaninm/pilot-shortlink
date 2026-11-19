-- OAuth linked accounts and single-use PKCE/state storage (plan pilot-oauth, requirements section 4).
-- oauth_states stores only the SHA-256 hex of state; rows are consumed atomically with DELETE ... RETURNING.

-- +goose Up
CREATE TABLE oauth_accounts (
    provider   text NOT NULL CHECK (provider IN ('google', 'github')),
    subject    text NOT NULL,
    user_id    bigint NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (provider, subject)
);
CREATE INDEX oauth_accounts_user_id_idx ON oauth_accounts (user_id);

CREATE TABLE oauth_states (
    state_hash text PRIMARY KEY,
    provider   text NOT NULL,
    verifier   text NOT NULL,
    expires_at timestamptz NOT NULL
);
CREATE INDEX oauth_states_expires_idx ON oauth_states (expires_at);

-- +goose Down
DROP INDEX oauth_states_expires_idx;
DROP TABLE oauth_states;
DROP TABLE oauth_accounts;
