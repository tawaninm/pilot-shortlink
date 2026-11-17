package db_test

import (
	"context"
	"strings"
	"testing"

	"github.com/tawaninm/pilot-shortlink/backend-b/internal/db"
	"github.com/tawaninm/pilot-shortlink/backend-b/internal/testdb"
)

func TestSchema(t *testing.T) {
	ctx := context.Background()
	pool := testdb.New(t)

	t.Run("all five tables exist", func(t *testing.T) {
		var n int
		err := pool.QueryRow(ctx, `SELECT count(*) FROM information_schema.tables
			WHERE table_schema = 'public' AND table_name IN ('users','sessions','links','click_daily','rate_limits')`).Scan(&n)
		if err != nil || n != 5 {
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

	t.Run("down then up works (level 2: every migration has a down)", func(t *testing.T) {
		if err := db.Migrate(ctx, pool, "down"); err != nil {
			t.Fatalf("down: %v", err)
		}
		if err := db.Migrate(ctx, pool, "up"); err != nil {
			t.Fatalf("up again: %v", err)
		}
	})
}

func mustScan(t *testing.T, row interface{ Scan(...any) error }, dst ...any) {
	t.Helper()
	if err := row.Scan(dst...); err != nil {
		t.Fatal(err)
	}
}
