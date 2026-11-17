-- Queries for sqlc. Every query on links filters by user_id on the server (docs/architecture.md section 6).

-- name: CreateUser :one
INSERT INTO users (email, password_hash) VALUES ($1, $2) RETURNING id;

-- name: GetUserByEmail :one
SELECT id, password_hash FROM users WHERE email = $1;

-- name: CreateSession :exec
INSERT INTO sessions (token_hash, user_id, expires_at) VALUES ($1, $2, $3);

-- name: GetSessionUser :one
SELECT u.id, u.email
FROM sessions s JOIN users u ON u.id = s.user_id
WHERE s.token_hash = $1 AND s.expires_at > now();

-- name: DeleteSession :exec
DELETE FROM sessions WHERE token_hash = $1;

-- name: GetRateCount :one
SELECT count FROM rate_limits WHERE key = $1 AND window_start = $2;

-- name: BumpRate :one
-- Atomic increment for a fixed window; returns the count after this request.
INSERT INTO rate_limits (key, window_start, count) VALUES ($1, $2, 1)
ON CONFLICT (key, window_start) DO UPDATE SET count = rate_limits.count + 1
RETURNING count;

-- name: DeleteOldRates :exec
DELETE FROM rate_limits WHERE window_start < $1;
