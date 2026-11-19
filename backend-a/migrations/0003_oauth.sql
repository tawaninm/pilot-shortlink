-- Rollback (D1 has no down migrations):
-- DROP INDEX IF EXISTS idx_oauth_accounts_user_id;
-- DROP TABLE IF EXISTS oauth_states;
-- DROP TABLE IF EXISTS oauth_accounts;

CREATE TABLE oauth_accounts (
    provider TEXT NOT NULL CHECK (provider IN ('google', 'github')),
    subject TEXT NOT NULL,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (provider, subject)
);

CREATE INDEX idx_oauth_accounts_user_id ON oauth_accounts(user_id);

CREATE TABLE oauth_states (
    state_hash TEXT PRIMARY KEY,
    provider TEXT NOT NULL CHECK (provider IN ('google', 'github')),
    verifier TEXT NOT NULL,
    expires_at INTEGER NOT NULL
);
