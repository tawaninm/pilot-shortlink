package db_test

import (
	"context"
	"regexp"
	"slices"
	"strings"
	"testing"

	"github.com/tawaninm/pilot-shortlink/backend-b/internal/auth"
	"github.com/tawaninm/pilot-shortlink/backend-b/internal/db"
	"github.com/tawaninm/pilot-shortlink/backend-b/internal/testdb"
)

func TestSchema(t *testing.T) {
	ctx := context.Background()
	pool := testdb.New(t)

	t.Run("all tables exist", func(t *testing.T) {
		var n int
		err := pool.QueryRow(ctx, `SELECT count(*) FROM information_schema.tables
			WHERE table_schema = 'public' AND table_name IN ('users','sessions','links','click_daily','rate_limits','email_tokens','oauth_accounts','oauth_states')`).Scan(&n)
		if err != nil || n != 8 {
			t.Fatalf("tables: got %d, err %v", n, err)
		}
	})

	t.Run("AC9 click_daily has no IP or User-Agent column", func(t *testing.T) {
		rows, err := pool.Query(ctx, `SELECT column_name FROM information_schema.columns WHERE table_name = 'click_daily'`)
		if err != nil {
			t.Fatal(err)
		}
		defer rows.Close()
		var got []string
		for rows.Next() {
			var c string
			if err := rows.Scan(&c); err != nil {
				t.Fatal(err)
			}
			got = append(got, c)
			if lc := strings.ToLower(c); strings.Contains(lc, "ip") || strings.Contains(lc, "agent") {
				t.Errorf("click_daily must not have column %q", c)
			}
		}
		if len(got) != 4 {
			t.Errorf("click_daily columns = %v, want link_id, day, referrer_host, clicks", got)
		}
	})

	t.Run("AC-V9 email_tokens has token_hash and no plain token column", func(t *testing.T) {
		rows, err := pool.Query(ctx, `SELECT column_name FROM information_schema.columns WHERE table_schema = 'public' AND table_name = 'email_tokens'`)
		if err != nil {
			t.Fatal(err)
		}
		defer rows.Close()
		var got []string
		for rows.Next() {
			var c string
			if err := rows.Scan(&c); err != nil {
				t.Fatal(err)
			}
			got = append(got, c)
			if lc := strings.ToLower(c); lc == "token" || (strings.Contains(lc, "token") && lc != "token_hash") {
				t.Errorf("email_tokens must not have plain token column %q", c)
			}
		}
		if !slices.Contains(got, "token_hash") || len(got) != 5 {
			t.Fatalf("email_tokens columns = %v, want token_hash, user_id, purpose, expires_at, used_at", got)
		}

		var uid int64
		mustScan(t, pool.QueryRow(ctx, `INSERT INTO users (email, password_hash) VALUES ('tok@example.test','x') RETURNING id`), &uid)
		tok, hash, err := auth.NewSessionToken()
		if err != nil {
			t.Fatal(err)
		}
		if _, err := pool.Exec(ctx, `INSERT INTO email_tokens (token_hash, user_id, purpose, expires_at) VALUES ($1, $2, 'verify', now() + interval '30 minutes')`, hash, uid); err != nil {
			t.Fatal(err)
		}
		var stored string
		mustScan(t, pool.QueryRow(ctx, `SELECT token_hash FROM email_tokens WHERE user_id = $1`, uid), &stored)
		if !regexp.MustCompile(`^[0-9a-f]{64}$`).MatchString(stored) || stored == tok || strings.Contains(stored, tok) {
			t.Fatalf("stored token_hash = %q (len %d), want 64 hex chars without raw token", stored, len(stored))
		}
		if _, err := pool.Exec(ctx, `INSERT INTO email_tokens (token_hash, user_id, purpose, expires_at) VALUES ('other', $1, 'bad', now())`, uid); err == nil {
			t.Fatal("invalid purpose was accepted")
		}
		if _, err := pool.Exec(ctx, `DELETE FROM users WHERE id = $1`, uid); err != nil {
			t.Fatal(err)
		}
		var left int
		mustScan(t, pool.QueryRow(ctx, `SELECT count(*) FROM email_tokens WHERE user_id = $1`, uid), &left)
		if left != 0 {
			t.Fatalf("email_tokens left after user delete cascade: %d", left)
		}
	})

	t.Run("deleting a user removes their links and clicks", func(t *testing.T) {
		var uid, lid int64
		mustScan(t, pool.QueryRow(ctx, `INSERT INTO users (email, password_hash) VALUES ('a@example.test','x') RETURNING id`), &uid)
		mustScan(t, pool.QueryRow(ctx, `INSERT INTO links (user_id, code, url) VALUES ($1,'abc1234','https://example.com') RETURNING id`, uid), &lid)
		if _, err := pool.Exec(ctx, `INSERT INTO click_daily (link_id, day, clicks) VALUES ($1, current_date, 1)`, lid); err != nil {
			t.Fatal(err)
		}
		if _, err := pool.Exec(ctx, `DELETE FROM users WHERE id = $1`, uid); err != nil {
			t.Fatal(err)
		}
		var n int
		mustScan(t, pool.QueryRow(ctx, `SELECT (SELECT count(*) FROM links) + (SELECT count(*) FROM click_daily)`), &n)
		if n != 0 {
			t.Fatalf("rows left after cascade: %d", n)
		}
	})

	t.Run("upper-case email or code is rejected", func(t *testing.T) {
		if _, err := pool.Exec(ctx, `INSERT INTO users (email, password_hash) VALUES ('A@example.test','x')`); err == nil {
			t.Fatal("upper-case email was accepted")
		}
	})

	t.Run("oauth_accounts enforces provider check, PK and cascades on user delete", func(t *testing.T) {
		rows, err := pool.Query(ctx, `SELECT column_name FROM information_schema.columns WHERE table_schema = 'public' AND table_name = 'oauth_accounts'`)
		if err != nil {
			t.Fatal(err)
		}
		defer rows.Close()
		var got []string
		for rows.Next() {
			var c string
			if err := rows.Scan(&c); err != nil {
				t.Fatal(err)
			}
			got = append(got, c)
		}
		for _, want := range []string{"provider", "subject", "user_id", "created_at"} {
			if !slices.Contains(got, want) {
				t.Fatalf("oauth_accounts missing column %q (got %v)", want, got)
			}
		}
		if len(got) != 4 {
			t.Fatalf("oauth_accounts columns = %v, want 4", got)
		}

		var uid int64
		mustScan(t, pool.QueryRow(ctx, `INSERT INTO users (email, password_hash) VALUES ('oa@example.test','!oauth') RETURNING id`), &uid)
		if _, err := pool.Exec(ctx, `INSERT INTO oauth_accounts (provider, subject, user_id) VALUES ('google', 'sub-1', $1)`, uid); err != nil {
			t.Fatal(err)
		}
		if _, err := pool.Exec(ctx, `INSERT INTO oauth_accounts (provider, subject, user_id) VALUES ('google', 'sub-1', $1)`, uid); err == nil {
			t.Fatal("duplicate (provider, subject) was accepted")
		}
		if _, err := pool.Exec(ctx, `INSERT INTO oauth_accounts (provider, subject, user_id) VALUES ('twitter', 'sub-2', $1)`, uid); err == nil {
			t.Fatal("invalid oauth provider was accepted")
		}
		if _, err := pool.Exec(ctx, `DELETE FROM users WHERE id = $1`, uid); err != nil {
			t.Fatal(err)
		}
		var left int
		mustScan(t, pool.QueryRow(ctx, `SELECT count(*) FROM oauth_accounts WHERE user_id = $1`, uid), &left)
		if left != 0 {
			t.Fatalf("oauth_accounts left after user delete cascade: %d", left)
		}
	})

	t.Run("oauth_states stores state_hash without plain state column", func(t *testing.T) {
		rows, err := pool.Query(ctx, `SELECT column_name FROM information_schema.columns WHERE table_schema = 'public' AND table_name = 'oauth_states'`)
		if err != nil {
			t.Fatal(err)
		}
		defer rows.Close()
		var got []string
		for rows.Next() {
			var c string
			if err := rows.Scan(&c); err != nil {
				t.Fatal(err)
			}
			got = append(got, c)
			if strings.ToLower(c) == "state" {
				t.Fatalf("oauth_states must not have plain state column %q", c)
			}
		}
		for _, want := range []string{"state_hash", "provider", "verifier", "expires_at"} {
			if !slices.Contains(got, want) {
				t.Fatalf("oauth_states missing column %q (got %v)", want, got)
			}
		}
		if len(got) != 4 {
			t.Fatalf("oauth_states columns = %v, want 4", got)
		}

		if _, err := pool.Exec(ctx, `INSERT INTO oauth_states (state_hash, provider, verifier, expires_at) VALUES ('h1', 'google', 'v1', now() + interval '10 minutes')`); err != nil {
			t.Fatal(err)
		}
		if _, err := pool.Exec(ctx, `INSERT INTO oauth_states (state_hash, provider, verifier, expires_at) VALUES ('h1', 'google', 'v2', now() + interval '10 minutes')`); err == nil {
			t.Fatal("duplicate state_hash was accepted")
		}
	})

	t.Run("down then up works (level 2: every migration has a down)", func(t *testing.T) {
		// Roll back 00003 (oauth) then 00002 (email_tokens), verify tables drop cleanly, then migrate back up.
		if err := db.Migrate(ctx, pool, "down"); err != nil {
			t.Fatalf("down 00003: %v", err)
		}
		var oauthTables int
		mustScan(t, pool.QueryRow(ctx, `SELECT count(*) FROM information_schema.tables WHERE table_schema = 'public' AND table_name IN ('oauth_accounts','oauth_states')`), &oauthTables)
		if oauthTables != 0 {
			t.Fatalf("00003 down left %d oauth tables", oauthTables)
		}
		if err := db.Migrate(ctx, pool, "down"); err != nil {
			t.Fatalf("down 00002: %v", err)
		}
		var uid int64
		mustScan(t, pool.QueryRow(ctx, `INSERT INTO users (email, password_hash) VALUES ('legacy@example.test','x') RETURNING id`), &uid)
		if err := db.Migrate(ctx, pool, "up"); err != nil {
			t.Fatalf("up again: %v", err)
		}
		var verified bool
		mustScan(t, pool.QueryRow(ctx, `SELECT email_verified_at = created_at FROM users WHERE id = $1`, uid), &verified)
		if !verified {
			t.Fatal("existing user was not marked verified by 00002 migration")
		}
		mustScan(t, pool.QueryRow(ctx, `SELECT count(*) FROM information_schema.tables WHERE table_schema = 'public' AND table_name IN ('oauth_accounts','oauth_states')`), &oauthTables)
		if oauthTables != 2 {
			t.Fatalf("up again: got %d oauth tables, want 2", oauthTables)
		}
	})
}

func mustScan(t *testing.T, row interface{ Scan(...any) error }, dst ...any) {
	t.Helper()
	if err := row.Scan(dst...); err != nil {
		t.Fatal(err)
	}
}
