//! Integration test for backend A's D1 SQLite schema (`migrations/0001_init.sql`).
//! Verifies all 5 tables exist, AC9 (no IP or User-Agent columns on `click_daily`),
//! and `ON DELETE CASCADE` integrity.

use pilot_shortlink_a::auth::{
    email_token_expires_at, generate_email_token, hash_email_token, CONSUME_EMAIL_TOKEN_SQL,
    DELETE_UNUSED_EMAIL_TOKENS_SQL, EMAIL_TOKEN_EXPIRY_SECS, INSERT_EMAIL_TOKEN_SQL,
    OAUTH_ONLY_PASSWORD_HASH,
};
use pilot_shortlink_a::mail::TokenPurpose;
use pilot_shortlink_a::oauth::{
    oauth_state_expires_at, Provider, CLAIM_UNVERIFIED_USER_SQL, CONSUME_OAUTH_STATE_SQL,
    DELETE_EXPIRED_OAUTH_STATES_SQL, INSERT_OAUTH_ACCOUNT_SQL, INSERT_OAUTH_STATE_SQL,
    OAUTH_STATE_TTL_SECS, SELECT_OAUTH_ACCOUNT_USER_SQL, WIPE_UNVERIFIED_USER_EMAIL_TOKENS_SQL,
    WIPE_UNVERIFIED_USER_SESSIONS_SQL,
};
use pilot_shortlink_a::session::{generate_session_token, hash_session_token};
use rusqlite::Connection;

const MIGRATION_0001: &str = include_str!("../migrations/0001_init.sql");
const MIGRATION_0002: &str = include_str!("../migrations/0002_email_tokens.sql");
const MIGRATION_0003: &str = include_str!("../migrations/0003_oauth.sql");

fn setup_db() -> Connection {
    let conn = Connection::open_in_memory().expect("open in-memory SQLite");
    conn.execute_batch("PRAGMA foreign_keys = ON;")
        .expect("enable foreign keys");
    conn.execute_batch(MIGRATION_0001)
        .expect("apply 0001_init.sql migration");
    conn.execute_batch(MIGRATION_0002)
        .expect("apply 0002_email_tokens.sql migration");
    conn.execute_batch(MIGRATION_0003)
        .expect("apply 0003_oauth.sql migration");
    conn
}

#[test]
fn migration_creates_all_five_tables() {
    let conn = setup_db();
    let expected = [
        "click_daily",
        "email_tokens",
        "links",
        "oauth_accounts",
        "oauth_states",
        "rate_limits",
        "sessions",
        "users",
    ];

    let mut stmt = conn
        .prepare(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
        )
        .expect("prepare table query");
    let tables: Vec<String> = stmt
        .query_map([], |row| row.get(0))
        .expect("query tables")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect table names");

    assert_eq!(tables, expected);
}

#[test]
fn ac9_click_daily_has_no_ip_or_user_agent_columns() {
    let conn = setup_db();
    let mut stmt = conn
        .prepare("PRAGMA table_info(click_daily)")
        .expect("prepare pragma table_info");
    let columns: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .expect("read columns")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect columns");

    assert!(!columns.is_empty(), "click_daily must have columns");
    for col in &columns {
        let lower = col.to_ascii_lowercase();
        assert!(
            !lower.contains("ip"),
            "AC9 violation: click_daily column '{col}' contains 'ip'"
        );
        assert!(
            !lower.contains("agent"),
            "AC9 violation: click_daily column '{col}' contains 'agent'"
        );
    }
    assert_eq!(columns, vec!["link_id", "day", "referrer_host", "clicks"]);
}

#[test]
fn foreign_keys_cascade_on_delete() {
    let conn = setup_db();

    conn.execute(
        "INSERT INTO users (id, email, password_hash) VALUES (1, 'a@example.com', 'hash')",
        [],
    )
    .expect("insert user");
    conn.execute(
        "INSERT INTO sessions (token_hash, user_id, expires_at) VALUES ('tok1', 1, 9999999999)",
        [],
    )
    .expect("insert session");
    conn.execute(
        "INSERT INTO links (id, user_id, code, url) VALUES (10, 1, 'abc1234', 'https://example.com')",
        [],
    )
    .expect("insert link");
    conn.execute(
        "INSERT INTO click_daily (link_id, day, referrer_host, clicks) VALUES (10, '2026-11-17', '', 3)",
        [],
    )
    .expect("insert click_daily");

    // Deleting the user must cascade to sessions, links, and click_daily.
    conn.execute("DELETE FROM users WHERE id = 1", [])
        .expect("delete user");

    let session_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM sessions", [], |r| r.get(0))
        .expect("count sessions");
    let link_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM links", [], |r| r.get(0))
        .expect("count links");
    let click_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM click_daily", [], |r| r.get(0))
        .expect("count clicks");

    assert_eq!(session_count, 0);
    assert_eq!(link_count, 0);
    assert_eq!(click_count, 0);
}

#[test]
fn auth_and_rate_limit_sql_queries_behave_atomically() {
    let conn = setup_db();

    // First insert returns the new user id.
    let first_id: Option<i64> = conn
        .query_row(
            "INSERT INTO users (email, password_hash) VALUES (?1, ?2) ON CONFLICT(email) DO NOTHING RETURNING id",
            ["alice@example.com", "phc1"],
            |r| r.get(0),
        )
        .ok();
    assert_eq!(first_id, Some(1));

    // Duplicate email returns no row instead of overwriting.
    let dup_id: Option<i64> = conn
        .query_row(
            "INSERT INTO users (email, password_hash) VALUES (?1, ?2) ON CONFLICT(email) DO NOTHING RETURNING id",
            ["alice@example.com", "phc2"],
            |r| r.get(0),
        )
        .ok();
    assert_eq!(dup_id, None);

    // Rate limit atomic upsert increments from 1 to 6.
    for expected in 1..=6 {
        let count: i64 = conn
            .query_row(
                "INSERT INTO rate_limits (key, window_start, count) VALUES (?1, ?2, 1) \
                 ON CONFLICT(key, window_start) DO UPDATE SET count = count + 1 \
                 RETURNING count",
                rusqlite::params!["login:alice@example.com", 1_800_000_i64],
                |r| r.get(0),
            )
            .expect("upsert rate_limits");
        assert_eq!(count, expected);
    }
}

#[test]
fn links_and_clicks_sql_queries_behave_atomically() {
    let conn = setup_db();

    conn.execute(
        "INSERT INTO users (id, email, password_hash) VALUES (1, 'a@example.com', 'h1'), (2, 'b@example.com', 'h2')",
        [],
    )
    .expect("insert users");

    // Atomic alias insert: first succeeds, second on same code returns None (AC4).
    let link_id: Option<i64> = conn
        .query_row(
            "INSERT INTO links (user_id, code, url) VALUES (?1, ?2, ?3) \
             ON CONFLICT(code) DO NOTHING \
             RETURNING id",
            rusqlite::params![1_i64, "tawan-cv", "https://example.com/cv"],
            |r| r.get(0),
        )
        .ok();
    assert_eq!(link_id, Some(1));

    let conflict_id: Option<i64> = conn
        .query_row(
            "INSERT INTO links (user_id, code, url) VALUES (?1, ?2, ?3) \
             ON CONFLICT(code) DO NOTHING \
             RETURNING id",
            rusqlite::params![2_i64, "tawan-cv", "https://evil.example/cv"],
            |r| r.get(0),
        )
        .ok();
    assert_eq!(conflict_id, None);

    // User 2 cannot update or delete User 1's link (AC7 / BOLA).
    let wrong_user_patch: Option<i64> = conn
        .query_row(
            "UPDATE links SET url = ?1 WHERE code = ?2 AND user_id = ?3 RETURNING id",
            rusqlite::params!["https://evil.example/", "tawan-cv", 2_i64],
            |r| r.get(0),
        )
        .ok();
    assert_eq!(wrong_user_patch, None);

    // 20 click upserts on the same (link_id, day, referrer_host) accumulate to 20.
    for _ in 0..20 {
        conn.execute(
            "INSERT INTO click_daily (link_id, day, referrer_host, clicks) \
             VALUES (?1, ?2, ?3, 1) \
             ON CONFLICT(link_id, day, referrer_host) DO UPDATE SET clicks = clicks + 1",
            rusqlite::params![1_i64, "2026-11-17", "www.facebook.com"],
        )
        .expect("upsert click_daily");
    }

    let total: i64 = conn
        .query_row(
            "SELECT COALESCE(SUM(clicks), 0) FROM click_daily WHERE link_id = ?1",
            [1_i64],
            |r| r.get(0),
        )
        .expect("sum clicks");
    assert_eq!(total, 20);
}

#[test]
fn ac_v9_email_tokens_has_token_hash_and_no_plain_token_column() {
    let conn = setup_db();
    let mut stmt = conn
        .prepare("PRAGMA table_info(email_tokens)")
        .expect("prepare pragma table_info(email_tokens)");
    let columns: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .expect("read email_tokens columns")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect email_tokens columns");

    assert!(
        columns.contains(&"token_hash".to_string()),
        "AC-V9: email_tokens must have token_hash column"
    );
    for col in &columns {
        let lower = col.to_ascii_lowercase();
        assert_ne!(
            lower, "token",
            "AC-V9 violation: email_tokens has a plain 'token' column"
        );
        assert!(
            !lower.contains("plain") && !lower.contains("raw"),
            "AC-V9 violation: email_tokens column '{col}' suggests plain token storage"
        );
    }
    assert_eq!(
        columns,
        vec!["token_hash", "user_id", "purpose", "expires_at", "used_at"]
    );

    // Verify 0002_email_tokens.sql has a rollback SQL comment at the top.
    assert!(
        MIGRATION_0002
            .lines()
            .take(5)
            .any(|line| line.trim_start().starts_with("--")
                && line.to_ascii_lowercase().contains("rollback")),
        "0002_email_tokens.sql must include rollback SQL in a comment at the top"
    );

    // Verify existing users before 0002 get email_verified_at = created_at, while new users get NULL.
    let pre_conn = Connection::open_in_memory().expect("open in-memory SQLite");
    pre_conn
        .execute_batch("PRAGMA foreign_keys = ON;")
        .expect("enable foreign keys");
    pre_conn
        .execute_batch(MIGRATION_0001)
        .expect("apply 0001_init.sql");
    pre_conn
        .execute(
            "INSERT INTO users (id, email, password_hash, created_at) VALUES (1, 'legacy@example.com', 'h1', '2026-11-17T00:00:00.000Z')",
            [],
        )
        .expect("insert legacy user before 0002");
    pre_conn
        .execute_batch(MIGRATION_0002)
        .expect("apply 0002_email_tokens.sql");
    pre_conn
        .execute(
            "INSERT INTO users (id, email, password_hash) VALUES (2, 'fresh@example.com', 'h2')",
            [],
        )
        .expect("insert fresh user after 0002");

    let legacy_verified: Option<String> = pre_conn
        .query_row(
            "SELECT email_verified_at FROM users WHERE id = 1",
            [],
            |r| r.get(0),
        )
        .expect("query legacy user");
    assert_eq!(
        legacy_verified.as_deref(),
        Some("2026-11-17T00:00:00.000Z"),
        "existing rows before 0002 must have email_verified_at = created_at"
    );

    let fresh_verified: Option<String> = pre_conn
        .query_row(
            "SELECT email_verified_at FROM users WHERE id = 2",
            [],
            |r| r.get(0),
        )
        .expect("query fresh user");
    assert_eq!(
        fresh_verified, None,
        "new rows after 0002 must start with email_verified_at = NULL"
    );
}

#[test]
fn email_tokens_atomic_consumption_and_constraints() {
    let conn = setup_db();

    conn.execute(
        "INSERT INTO users (id, email, password_hash) VALUES (1, 'a@example.com', 'h1')",
        [],
    )
    .expect("insert user");

    // CHECK (purpose IN ('verify', 'reset')) rejects unknown purposes.
    let bad_purpose = conn.execute(
        "INSERT INTO email_tokens (token_hash, user_id, purpose, expires_at) VALUES (?1, 1, 'other', 1000)",
        ["0".repeat(64)],
    );
    assert!(
        bad_purpose.is_err(),
        "purpose CHECK constraint must reject 'other'"
    );

    // Issue a real verify token using the same generator, hash, expiry constant, and SQL as auth.rs.
    let raw_token = generate_email_token().expect("generate email token");
    let token_hash = hash_email_token(&raw_token);
    let t0 = 1_700_000_000_i64;
    let expires_at = email_token_expires_at(t0);
    assert_eq!(expires_at, t0 + EMAIL_TOKEN_EXPIRY_SECS);

    conn.execute(
        DELETE_UNUSED_EMAIL_TOKENS_SQL,
        rusqlite::params![1_i64, TokenPurpose::Verify.as_str()],
    )
    .expect("delete prior unused verify tokens");
    conn.execute(
        INSERT_EMAIL_TOKEN_SQL,
        rusqlite::params![token_hash, 1_i64, TokenPurpose::Verify.as_str(), expires_at],
    )
    .expect("insert verify token");

    let stored_hash: String = conn
        .query_row(
            "SELECT token_hash FROM email_tokens WHERE user_id = 1",
            [],
            |r| r.get(0),
        )
        .expect("select stored token_hash");
    assert_eq!(stored_hash.len(), 64);
    assert_ne!(stored_hash, raw_token);
    assert!(stored_hash
        .chars()
        .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));

    // Expired attempts (at exact 30-minute boundary t0 + EMAIL_TOKEN_EXPIRY_SECS and +1s) fail to consume.
    for expired_now in [expires_at, expires_at + 1] {
        let expired_attempt: Option<i64> = conn
            .query_row(
                CONSUME_EMAIL_TOKEN_SQL,
                rusqlite::params![expired_now, token_hash, TokenPurpose::Verify.as_str()],
                |r| r.get(0),
            )
            .ok();
        assert_eq!(
            expired_attempt, None,
            "token at or past expiry must not be consumed"
        );
    }

    // Wrong purpose attempt ('reset') fails to consume.
    let wrong_purpose: Option<i64> = conn
        .query_row(
            CONSUME_EMAIL_TOKEN_SQL,
            rusqlite::params![t0 + 100, token_hash, TokenPurpose::Reset.as_str()],
            |r| r.get(0),
        )
        .ok();
    assert_eq!(wrong_purpose, None);

    // Valid attempt 1 second before expiry consumes the token and returns user_id = 1.
    let first_consume: Option<i64> = conn
        .query_row(
            CONSUME_EMAIL_TOKEN_SQL,
            rusqlite::params![expires_at - 1, token_hash, TokenPurpose::Verify.as_str()],
            |r| r.get(0),
        )
        .ok();
    assert_eq!(first_consume, Some(1));

    // Second attempt with the same token fails because used_at IS NOT NULL.
    let second_consume: Option<i64> = conn
        .query_row(
            CONSUME_EMAIL_TOKEN_SQL,
            rusqlite::params![t0 + 101, token_hash, TokenPurpose::Verify.as_str()],
            |r| r.get(0),
        )
        .ok();
    assert_eq!(second_consume, None);

    // Issuing a new reset token deletes an earlier unused reset token via DELETE_UNUSED_EMAIL_TOKENS_SQL.
    let t1 = t0 + 200;
    let first_reset_hash = hash_email_token(&generate_email_token().expect("first reset token"));
    let second_reset_hash = hash_email_token(&generate_email_token().expect("second reset token"));
    conn.execute(
        INSERT_EMAIL_TOKEN_SQL,
        rusqlite::params![
            first_reset_hash,
            1_i64,
            TokenPurpose::Reset.as_str(),
            email_token_expires_at(t1)
        ],
    )
    .expect("insert first reset token");
    conn.execute(
        DELETE_UNUSED_EMAIL_TOKENS_SQL,
        rusqlite::params![1_i64, TokenPurpose::Reset.as_str()],
    )
    .expect("delete prior unused reset token");
    conn.execute(
        INSERT_EMAIL_TOKEN_SQL,
        rusqlite::params![
            second_reset_hash,
            1_i64,
            TokenPurpose::Reset.as_str(),
            email_token_expires_at(t1)
        ],
    )
    .expect("insert second reset token");

    let superseded: Option<i64> = conn
        .query_row(
            CONSUME_EMAIL_TOKEN_SQL,
            rusqlite::params![t1 + 1, first_reset_hash, TokenPurpose::Reset.as_str()],
            |r| r.get(0),
        )
        .ok();
    assert_eq!(superseded, None, "superseded reset token must be deleted");

    let latest_reset: Option<i64> = conn
        .query_row(
            CONSUME_EMAIL_TOKEN_SQL,
            rusqlite::params![t1 + 1, second_reset_hash, TokenPurpose::Reset.as_str()],
            |r| r.get(0),
        )
        .ok();
    assert_eq!(latest_reset, Some(1));

    // Deleting user cascades to email_tokens.
    conn.execute("DELETE FROM users WHERE id = 1", [])
        .expect("delete user");
    let remaining: i64 = conn
        .query_row("SELECT COUNT(*) FROM email_tokens", [], |r| r.get(0))
        .expect("count email_tokens");
    assert_eq!(remaining, 0);
}

#[test]
fn oauth_accounts_and_states_schema_constraints_and_atomic_expiry() {
    // 0003_oauth.sql must include rollback SQL in a comment at the top.
    assert!(
        MIGRATION_0003
            .lines()
            .take(5)
            .any(|line| line.trim_start().starts_with("--")
                && line.to_ascii_lowercase().contains("rollback")),
        "0003_oauth.sql must include rollback SQL in a comment at the top"
    );

    let conn = setup_db();

    // Check oauth_accounts columns and composite PK (provider, subject).
    let mut acct_stmt = conn
        .prepare("PRAGMA table_info(oauth_accounts)")
        .expect("pragma oauth_accounts");
    let acct_info: Vec<(String, i64)> = acct_stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(1)?, row.get::<_, i64>(5)?))
        })
        .expect("query oauth_accounts info")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect oauth_accounts info");
    let acct_cols: Vec<String> = acct_info.iter().map(|(c, _)| c.clone()).collect();
    assert_eq!(
        acct_cols,
        vec!["provider", "subject", "user_id", "created_at"]
    );
    let acct_pks: Vec<(String, i64)> = acct_info.into_iter().filter(|(_, pk)| *pk > 0).collect();
    assert_eq!(
        acct_pks,
        vec![("provider".to_string(), 1), ("subject".to_string(), 2)]
    );

    // Check index on oauth_accounts(user_id).
    let idx_exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND tbl_name = 'oauth_accounts' AND name = 'idx_oauth_accounts_user_id'",
            [],
            |r| r.get(0),
        )
        .expect("query oauth_accounts index");
    assert_eq!(idx_exists, 1);

    // Check oauth_states columns and PK (state_hash).
    let mut state_stmt = conn
        .prepare("PRAGMA table_info(oauth_states)")
        .expect("pragma oauth_states");
    let state_info: Vec<(String, i64)> = state_stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(1)?, row.get::<_, i64>(5)?))
        })
        .expect("query oauth_states info")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect oauth_states info");
    let state_cols: Vec<String> = state_info.iter().map(|(c, _)| c.clone()).collect();
    assert_eq!(
        state_cols,
        vec!["state_hash", "provider", "verifier", "expires_at"]
    );
    let state_pks: Vec<(String, i64)> = state_info.into_iter().filter(|(_, pk)| *pk > 0).collect();
    assert_eq!(state_pks, vec![("state_hash".to_string(), 1)]);

    // Insert user and check provider CHECK constraint + duplicate (provider, subject) PK + cascade delete.
    conn.execute(
        "INSERT INTO users (id, email, password_hash, email_verified_at) VALUES (1, 'oauth@example.com', '!oauth', '2026-11-19T00:00:00.000Z')",
        [],
    )
    .expect("insert oauth user");

    let bad_provider = conn.execute(
        "INSERT INTO oauth_accounts (provider, subject, user_id, created_at) VALUES ('twitter', 'sub1', 1, 1700000000)",
        [],
    );
    assert!(
        bad_provider.is_err(),
        "oauth_accounts CHECK constraint must reject providers other than google/github"
    );

    conn.execute(
        "INSERT INTO oauth_accounts (provider, subject, user_id, created_at) VALUES ('google', 'sub1', 1, 1700000000)",
        [],
    )
    .expect("insert google link");

    let dup_pk = conn.execute(
        "INSERT INTO oauth_accounts (provider, subject, user_id, created_at) VALUES ('google', 'sub1', 1, 1700000001)",
        [],
    );
    assert!(
        dup_pk.is_err(),
        "duplicate (provider, subject) must violate PK"
    );

    // Deleting user cascades to oauth_accounts.
    conn.execute("DELETE FROM users WHERE id = 1", [])
        .expect("delete user");
    let acct_remaining: i64 = conn
        .query_row("SELECT COUNT(*) FROM oauth_accounts", [], |r| r.get(0))
        .expect("count oauth_accounts");
    assert_eq!(acct_remaining, 0);

    // Exercise oauth_states atomic single-use consumption & expiry with an injected clock.
    let raw_state = generate_session_token().expect("generate state");
    let state_hash = hash_session_token(&raw_state);
    let verifier = generate_session_token().expect("generate verifier");
    let t0 = 1_700_000_000_i64;
    let expires_at = oauth_state_expires_at(t0);
    assert_eq!(expires_at, t0 + OAUTH_STATE_TTL_SECS);

    conn.execute(
        INSERT_OAUTH_STATE_SQL,
        rusqlite::params![state_hash, Provider::Google.as_str(), verifier, expires_at],
    )
    .expect("insert oauth state");

    // At or past expiry (t0 + 600, t0 + 601), state is NOT consumed.
    for expired_now in [expires_at, expires_at + 1] {
        let res: Option<String> = conn
            .query_row(
                CONSUME_OAUTH_STATE_SQL,
                rusqlite::params![state_hash, Provider::Google.as_str(), expired_now],
                |r| r.get(0),
            )
            .ok();
        assert_eq!(res, None, "expired oauth_state must not be consumed");
    }

    // Wrong provider ('github') does NOT consume a 'google' state.
    let wrong_provider: Option<String> = conn
        .query_row(
            CONSUME_OAUTH_STATE_SQL,
            rusqlite::params![state_hash, Provider::Github.as_str(), t0 + 10],
            |r| r.get(0),
        )
        .ok();
    assert_eq!(wrong_provider, None);

    // Valid consume 1 second before expiry returns the verifier and deletes the row atomically.
    let consumed: Option<String> = conn
        .query_row(
            CONSUME_OAUTH_STATE_SQL,
            rusqlite::params![state_hash, Provider::Google.as_str(), expires_at - 1],
            |r| r.get(0),
        )
        .ok();
    assert_eq!(consumed.as_deref(), Some(verifier.as_str()));

    // Replay of the same state fails immediately because the row was deleted.
    let replay: Option<String> = conn
        .query_row(
            CONSUME_OAUTH_STATE_SQL,
            rusqlite::params![state_hash, Provider::Google.as_str(), t0 + 11],
            |r| r.get(0),
        )
        .ok();
    assert_eq!(replay, None, "replayed oauth_state must not succeed");

    // Opportunistic expired state cleanup deletes rows with expires_at <= now.
    let old_hash = hash_session_token(&generate_session_token().expect("old state"));
    conn.execute(
        INSERT_OAUTH_STATE_SQL,
        rusqlite::params![old_hash, Provider::Github.as_str(), "v1", t0 + 100],
    )
    .expect("insert old state");
    conn.execute(DELETE_EXPIRED_OAUTH_STATES_SQL, rusqlite::params![t0 + 100])
        .expect("cleanup expired states");
    let states_left: i64 = conn
        .query_row("SELECT COUNT(*) FROM oauth_states", [], |r| r.get(0))
        .expect("count oauth_states");
    assert_eq!(states_left, 0);
}

#[test]
fn ac_o12_pre_hijack_sql_wipes_unverified_state_and_preserves_verified_state() {
    let conn = setup_db();

    // User 1: unverified squatter account with password, active session, unused + used email tokens.
    conn.execute(
        "INSERT INTO users (id, email, password_hash, email_verified_at) VALUES (1, 'victim@example.com', 'squatter-phc', NULL)",
        [],
    )
    .expect("insert unverified user");
    conn.execute(
        "INSERT INTO sessions (token_hash, user_id, expires_at) VALUES ('sess-unverified', 1, 9999999999)",
        [],
    )
    .expect("insert squatter session");
    conn.execute(
        "INSERT INTO email_tokens (token_hash, user_id, purpose, expires_at, used_at) VALUES ('tok-unused', 1, 'verify', 9999999999, NULL), ('tok-used', 1, 'verify', 9999999999, 1700000000)",
        [],
    )
    .expect("insert squatter tokens");

    // User 2: verified account with password, active session, unused email token.
    conn.execute(
        "INSERT INTO users (id, email, password_hash, email_verified_at) VALUES (2, 'verified@example.com', 'owner-phc', '2026-11-19T00:00:00.000Z')",
        [],
    )
    .expect("insert verified user");
    conn.execute(
        "INSERT INTO sessions (token_hash, user_id, expires_at) VALUES ('sess-verified', 2, 9999999999)",
        [],
    )
    .expect("insert owner session");
    conn.execute(
        "INSERT INTO email_tokens (token_hash, user_id, purpose, expires_at, used_at) VALUES ('tok-owner-unused', 2, 'reset', 9999999999, NULL)",
        [],
    )
    .expect("insert owner token");

    // Run F1 + F3 batch on unverified user 1.
    conn.execute(
        INSERT_OAUTH_ACCOUNT_SQL,
        rusqlite::params![
            Provider::Google.as_str(),
            "sub-victim",
            1_i64,
            1_700_000_000_i64
        ],
    )
    .expect("link oauth account for user 1");
    conn.execute(WIPE_UNVERIFIED_USER_SESSIONS_SQL, rusqlite::params![1_i64])
        .expect("wipe squatter sessions");
    conn.execute(
        WIPE_UNVERIFIED_USER_EMAIL_TOKENS_SQL,
        rusqlite::params![1_i64],
    )
    .expect("wipe squatter unused email tokens");
    conn.execute(
        CLAIM_UNVERIFIED_USER_SQL,
        rusqlite::params![OAUTH_ONLY_PASSWORD_HASH, 1_i64],
    )
    .expect("claim unverified user 1");

    let resolved_u1: i64 = conn
        .query_row(
            SELECT_OAUTH_ACCOUNT_USER_SQL,
            rusqlite::params![Provider::Google.as_str(), "sub-victim"],
            |r| r.get(0),
        )
        .expect("re-read oauth_accounts user_id");
    assert_eq!(resolved_u1, 1);

    let (u1_hash, u1_verified): (String, Option<String>) = conn
        .query_row(
            "SELECT password_hash, email_verified_at FROM users WHERE id = 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("query user 1");
    assert_eq!(u1_hash, OAUTH_ONLY_PASSWORD_HASH);
    assert!(u1_verified.is_some());

    let u1_sessions: i64 = conn
        .query_row("SELECT COUNT(*) FROM sessions WHERE user_id = 1", [], |r| {
            r.get(0)
        })
        .expect("count user 1 sessions");
    assert_eq!(u1_sessions, 0);

    let u1_unused_tokens: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM email_tokens WHERE user_id = 1 AND used_at IS NULL",
            [],
            |r| r.get(0),
        )
        .expect("count user 1 unused tokens");
    assert_eq!(u1_unused_tokens, 0);

    let u1_used_tokens: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM email_tokens WHERE user_id = 1 AND used_at IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .expect("count user 1 used tokens");
    assert_eq!(u1_used_tokens, 1);

    // Even if the wipe/claim SQL were executed against verified user 2, the email_verified_at IS NULL guard keeps it intact.
    conn.execute(
        INSERT_OAUTH_ACCOUNT_SQL,
        rusqlite::params![
            Provider::Github.as_str(),
            "sub-owner",
            2_i64,
            1_700_000_000_i64
        ],
    )
    .expect("link oauth account for user 2");
    conn.execute(WIPE_UNVERIFIED_USER_SESSIONS_SQL, rusqlite::params![2_i64])
        .expect("no-op wipe sessions on verified user 2");
    conn.execute(
        WIPE_UNVERIFIED_USER_EMAIL_TOKENS_SQL,
        rusqlite::params![2_i64],
    )
    .expect("no-op wipe tokens on verified user 2");
    conn.execute(
        CLAIM_UNVERIFIED_USER_SQL,
        rusqlite::params![OAUTH_ONLY_PASSWORD_HASH, 2_i64],
    )
    .expect("no-op claim on verified user 2");

    let (u2_hash, u2_verified): (String, Option<String>) = conn
        .query_row(
            "SELECT password_hash, email_verified_at FROM users WHERE id = 2",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("query user 2");
    assert_eq!(u2_hash, "owner-phc");
    assert_eq!(u2_verified.as_deref(), Some("2026-11-19T00:00:00.000Z"));

    let u2_sessions: i64 = conn
        .query_row("SELECT COUNT(*) FROM sessions WHERE user_id = 2", [], |r| {
            r.get(0)
        })
        .expect("count user 2 sessions");
    assert_eq!(u2_sessions, 1);

    let u2_unused_tokens: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM email_tokens WHERE user_id = 2 AND used_at IS NULL",
            [],
            |r| r.get(0),
        )
        .expect("count user 2 unused tokens");
    assert_eq!(u2_unused_tokens, 1);

    // F3: duplicate INSERT ... ON CONFLICT DO NOTHING keeps original user_id, and re-read returns original owner.
    conn.execute(
        INSERT_OAUTH_ACCOUNT_SQL,
        rusqlite::params![
            Provider::Google.as_str(),
            "sub-victim",
            2_i64,
            1_700_000_001_i64
        ],
    )
    .expect("conflict do nothing");
    let still_u1: i64 = conn
        .query_row(
            SELECT_OAUTH_ACCOUNT_USER_SQL,
            rusqlite::params![Provider::Google.as_str(), "sub-victim"],
            |r| r.get(0),
        )
        .expect("re-read after conflict");
    assert_eq!(still_u1, 1);
}
