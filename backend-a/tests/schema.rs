//! Integration test for backend A's D1 SQLite schema (`migrations/0001_init.sql`).
//! Verifies all 5 tables exist, AC9 (no IP or User-Agent columns on `click_daily`),
//! and `ON DELETE CASCADE` integrity.

use rusqlite::Connection;

const MIGRATION_0001: &str = include_str!("../migrations/0001_init.sql");

fn setup_db() -> Connection {
    let conn = Connection::open_in_memory().expect("open in-memory SQLite");
    conn.execute_batch("PRAGMA foreign_keys = ON;")
        .expect("enable foreign keys");
    conn.execute_batch(MIGRATION_0001)
        .expect("apply 0001_init.sql migration");
    conn
}

#[test]
fn migration_creates_all_five_tables() {
    let conn = setup_db();
    let expected = ["click_daily", "links", "rate_limits", "sessions", "users"];

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
    assert_eq!(
        columns,
        vec!["link_id", "day", "referrer_host", "clicks"]
    );
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
