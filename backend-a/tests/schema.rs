//! Integration test for backend A's D1 SQLite schema (`migrations/0001_init.sql`).
//! Verifies all 5 tables exist, AC9 (no IP or User-Agent columns on `click_daily`),
//! and `ON DELETE CASCADE` integrity.

use rusqlite::Connection;

const MIGRATION_0001: &str = include_str!("../migrations/0001_init.sql");
const MIGRATION_0002: &str = include_str!("../migrations/0002_email_tokens.sql");

fn setup_db() -> Connection {
    let conn = Connection::open_in_memory().expect("open in-memory SQLite");
    conn.execute_batch("PRAGMA foreign_keys = ON;")
        .expect("enable foreign keys");
    conn.execute_batch(MIGRATION_0001)
        .expect("apply 0001_init.sql migration");
    conn.execute_batch(MIGRATION_0002)
        .expect("apply 0002_email_tokens.sql migration");
    conn
}

#[test]
fn migration_creates_all_five_tables() {
    let conn = setup_db();
    let expected = [
        "click_daily",
        "email_tokens",
        "links",
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

    // Store a 64-char SHA-256 hex token_hash (never a 43-char plain token).
    let token_hash = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let t0 = 1_700_000_000_i64;
    let expires_at = t0 + 1_800;

    conn.execute(
        "INSERT INTO email_tokens (token_hash, user_id, purpose, expires_at) VALUES (?1, 1, 'verify', ?2)",
        rusqlite::params![token_hash, expires_at],
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
    assert!(stored_hash
        .chars()
        .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));

    // Expired attempt (at t0 + 1800) fails to consume and does not mark used_at.
    let expired_attempt: Option<i64> = conn
        .query_row(
            "UPDATE email_tokens SET used_at = ?1 \
             WHERE token_hash = ?2 AND purpose = ?3 AND used_at IS NULL AND expires_at > ?1 \
             RETURNING user_id",
            rusqlite::params![expires_at, token_hash, "verify"],
            |r| r.get(0),
        )
        .ok();
    assert_eq!(expired_attempt, None);

    // Wrong purpose attempt ('reset') fails to consume.
    let wrong_purpose: Option<i64> = conn
        .query_row(
            "UPDATE email_tokens SET used_at = ?1 \
             WHERE token_hash = ?2 AND purpose = ?3 AND used_at IS NULL AND expires_at > ?1 \
             RETURNING user_id",
            rusqlite::params![t0 + 100, token_hash, "reset"],
            |r| r.get(0),
        )
        .ok();
    assert_eq!(wrong_purpose, None);

    // Valid attempt before expiry consumes the token and returns user_id = 1.
    let first_consume: Option<i64> = conn
        .query_row(
            "UPDATE email_tokens SET used_at = ?1 \
             WHERE token_hash = ?2 AND purpose = ?3 AND used_at IS NULL AND expires_at > ?1 \
             RETURNING user_id",
            rusqlite::params![t0 + 100, token_hash, "verify"],
            |r| r.get(0),
        )
        .ok();
    assert_eq!(first_consume, Some(1));

    // Second attempt with the same token fails because used_at IS NOT NULL.
    let second_consume: Option<i64> = conn
        .query_row(
            "UPDATE email_tokens SET used_at = ?1 \
             WHERE token_hash = ?2 AND purpose = ?3 AND used_at IS NULL AND expires_at > ?1 \
             RETURNING user_id",
            rusqlite::params![t0 + 101, token_hash, "verify"],
            |r| r.get(0),
        )
        .ok();
    assert_eq!(second_consume, None);

    // Deleting user cascades to email_tokens.
    conn.execute("DELETE FROM users WHERE id = 1", [])
        .expect("delete user");
    let remaining: i64 = conn
        .query_row("SELECT COUNT(*) FROM email_tokens", [], |r| r.get(0))
        .expect("count email_tokens");
    assert_eq!(remaining, 0);
}
