-- Initial schema for pilot-shortlink backend A (Cloudflare D1 / SQLite).
-- Matches docs/architecture.md section 5.
-- Privacy requirement (AC9): click_daily aggregates daily clicks by UTC date and referrer_host only;
-- it never stores IP addresses or User-Agent strings.

CREATE TABLE users (
    id INTEGER PRIMARY KEY,
    email TEXT NOT NULL UNIQUE CHECK (email = lower(email)),
    password_hash TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE TABLE sessions (
    token_hash TEXT NOT NULL PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    expires_at INTEGER NOT NULL
);

CREATE INDEX idx_sessions_user_id ON sessions(user_id);

CREATE TABLE links (
    id INTEGER PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    code TEXT NOT NULL UNIQUE CHECK (code = lower(code)),
    url TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX idx_links_user_id_created_at ON links(user_id, created_at DESC);

CREATE TABLE click_daily (
    link_id INTEGER NOT NULL REFERENCES links(id) ON DELETE CASCADE,
    day TEXT NOT NULL,
    referrer_host TEXT NOT NULL DEFAULT '',
    clicks INTEGER NOT NULL DEFAULT 0 CHECK (clicks >= 0),
    PRIMARY KEY (link_id, day, referrer_host)
);

CREATE TABLE rate_limits (
    key TEXT NOT NULL,
    window_start INTEGER NOT NULL,
    count INTEGER NOT NULL DEFAULT 0 CHECK (count >= 0),
    PRIMARY KEY (key, window_start)
);
