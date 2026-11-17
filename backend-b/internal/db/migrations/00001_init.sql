-- Schema for pilot-shortlink backend B (Postgres). Same tables as backend A (docs/architecture.md section 5).
-- click_daily keeps daily totals per referrer host only: no IP, no User-Agent (requirements section 4, AC9).

-- +goose Up
CREATE TABLE users (
    id            bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    email         text NOT NULL UNIQUE CHECK (email = lower(email)),
    password_hash text NOT NULL,
    created_at    timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE sessions (
    token_hash text PRIMARY KEY, -- SHA-256 hex of the cookie token; the token itself is never stored
    user_id    bigint NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    expires_at timestamptz NOT NULL
);
CREATE INDEX sessions_user_id_idx ON sessions (user_id);

CREATE TABLE links (
    id         bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    user_id    bigint NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    code       text NOT NULL UNIQUE CHECK (code = lower(code)),
    url        text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX links_user_created_idx ON links (user_id, created_at DESC, id DESC);

CREATE TABLE click_daily (
    link_id       bigint NOT NULL REFERENCES links (id) ON DELETE CASCADE,
    day           date NOT NULL,
    referrer_host text NOT NULL DEFAULT '', -- '' means unknown
    clicks        bigint NOT NULL DEFAULT 0 CHECK (clicks >= 0),
    PRIMARY KEY (link_id, day, referrer_host)
);

CREATE TABLE rate_limits (
    key          text NOT NULL,
    window_start timestamptz NOT NULL,
    count        integer NOT NULL DEFAULT 0 CHECK (count >= 0),
    PRIMARY KEY (key, window_start)
);

-- +goose Down
DROP TABLE rate_limits;
DROP TABLE click_daily;
DROP TABLE links;
DROP TABLE sessions;
DROP TABLE users;
