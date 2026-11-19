-- Queries for sqlc. Every query on links filters by user_id on the server (docs/architecture.md section 6).

-- name: CreateUser :one
INSERT INTO users (email, password_hash) VALUES ($1, $2) RETURNING id;

-- name: GetUserByEmail :one
SELECT id, password_hash, (email_verified_at IS NOT NULL)::boolean AS email_verified FROM users WHERE email = $1;

-- name: CreateSession :exec
INSERT INTO sessions (token_hash, user_id, expires_at) VALUES ($1, $2, $3);

-- name: GetSessionUser :one
SELECT u.id, u.email, (u.email_verified_at IS NOT NULL)::boolean AS email_verified
FROM sessions s JOIN users u ON u.id = s.user_id
WHERE s.token_hash = $1 AND s.expires_at > now();

-- name: DeleteSession :exec
DELETE FROM sessions WHERE token_hash = $1;

-- name: DeleteUserSessions :exec
DELETE FROM sessions WHERE user_id = $1;

-- name: DeleteUnusedEmailTokens :exec
DELETE FROM email_tokens WHERE user_id = $1 AND purpose = $2 AND used_at IS NULL;

-- name: CreateEmailToken :exec
INSERT INTO email_tokens (token_hash, user_id, purpose, expires_at) VALUES ($1, $2, $3, $4);

-- name: ConsumeEmailToken :one
-- Atomic single-statement consume: concurrent requests cannot both consume the same token (AC-V7).
UPDATE email_tokens SET used_at = now() WHERE token_hash = $1 AND purpose = $2 AND used_at IS NULL AND expires_at > now() RETURNING user_id;

-- name: MarkEmailVerified :exec
UPDATE users SET email_verified_at = COALESCE(email_verified_at, now()) WHERE id = $1;

-- name: ResetUserPassword :exec
UPDATE users SET password_hash = $2, email_verified_at = COALESCE(email_verified_at, now()) WHERE id = $1;

-- name: GetRateCount :one
SELECT count FROM rate_limits WHERE key = $1 AND window_start = $2;

-- name: BumpRate :one
-- Atomic increment for a fixed window; returns the count after this request.
INSERT INTO rate_limits (key, window_start, count) VALUES ($1, $2, 1)
ON CONFLICT (key, window_start) DO UPDATE SET count = rate_limits.count + 1
RETURNING count;

-- name: DeleteOldRates :exec
DELETE FROM rate_limits WHERE window_start < $1;

-- name: CreateLink :one
-- No check-then-insert: the UNIQUE(code) violation decides who gets an alias (AC4).
INSERT INTO links (user_id, code, url) VALUES ($1, $2, $3) RETURNING id, code, url, created_at;

-- name: ListLinks :many
SELECT l.id, l.code, l.url, l.created_at,
       (SELECT coalesce(sum(c.clicks), 0) FROM click_daily c WHERE c.link_id = l.id)::bigint AS clicks_total
FROM links l WHERE l.user_id = $1
ORDER BY l.created_at DESC, l.id DESC;

-- name: DailyClicksForUser :many
SELECT c.link_id, c.day, sum(c.clicks)::bigint AS clicks
FROM click_daily c JOIN links l ON l.id = c.link_id
WHERE l.user_id = $1 AND c.day >= $2
GROUP BY c.link_id, c.day;

-- name: GetOwnLink :one
SELECT id, code, url, created_at FROM links WHERE code = $1 AND user_id = $2;

-- name: UpdateOwnLinkURL :one
UPDATE links SET url = $3 WHERE code = $1 AND user_id = $2 RETURNING id, code, url, created_at;

-- name: DeleteOwnLink :execrows
DELETE FROM links WHERE code = $1 AND user_id = $2;

-- name: LinkTotal :one
SELECT coalesce(sum(clicks), 0)::bigint FROM click_daily WHERE link_id = $1;

-- name: LinkDaily :many
SELECT day, sum(clicks)::bigint AS clicks FROM click_daily WHERE link_id = $1 AND day >= $2 GROUP BY day;

-- name: LinkReferrers :many
SELECT referrer_host, sum(clicks)::bigint AS clicks FROM click_daily
WHERE link_id = $1 AND day >= $2
GROUP BY referrer_host ORDER BY clicks DESC, referrer_host LIMIT 10;

-- name: GetLinkTarget :one
SELECT id, url FROM links WHERE code = $1;

-- name: CountClick :exec
-- One atomic statement per click: concurrent visits never lose a count.
INSERT INTO click_daily (link_id, day, referrer_host, clicks) VALUES ($1, $2, $3, 1)
ON CONFLICT (link_id, day, referrer_host) DO UPDATE SET clicks = click_daily.clicks + 1;

-- name: ResetRate :exec
-- A successful login clears the failure count for that email (the limit counts guesses, not users).
DELETE FROM rate_limits WHERE key = $1;
