-- Rollback (D1 has no down migrations):
-- DROP INDEX IF EXISTS idx_email_tokens_user_id_purpose;
-- DROP TABLE IF EXISTS email_tokens;
-- ALTER TABLE users DROP COLUMN email_verified_at;

ALTER TABLE users ADD COLUMN email_verified_at TEXT NULL;

UPDATE users SET email_verified_at = created_at WHERE email_verified_at IS NULL;

CREATE TABLE email_tokens (
    token_hash TEXT PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    purpose TEXT NOT NULL CHECK (purpose IN ('verify', 'reset')),
    expires_at INTEGER NOT NULL,
    used_at INTEGER
);

CREATE INDEX idx_email_tokens_user_id_purpose ON email_tokens(user_id, purpose);
