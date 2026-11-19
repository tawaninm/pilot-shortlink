-- Email verification and password reset tokens (plan pilot-addons, requirements section 4).
-- Only the SHA-256 hex of a token is stored (AC-V9). Accounts existing before this migration are marked verified.

-- +goose Up
ALTER TABLE users ADD COLUMN email_verified_at timestamptz NULL;
UPDATE users SET email_verified_at = created_at WHERE email_verified_at IS NULL;

CREATE TABLE email_tokens (
    token_hash text PRIMARY KEY,
    user_id    bigint NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    purpose    text NOT NULL CHECK (purpose IN ('verify', 'reset')),
    expires_at timestamptz NOT NULL,
    used_at    timestamptz NULL
);
CREATE INDEX email_tokens_user_purpose_idx ON email_tokens (user_id, purpose);

-- +goose Down
DROP TABLE email_tokens;
ALTER TABLE users DROP COLUMN email_verified_at;
