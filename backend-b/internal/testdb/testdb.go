// Package testdb starts a throwaway Postgres in Docker (testcontainers-go) for integration tests.
package testdb

import (
	"context"
	"testing"

	"github.com/jackc/pgx/v5/pgxpool"
	"github.com/testcontainers/testcontainers-go"
	"github.com/testcontainers/testcontainers-go/modules/postgres"
	"github.com/tawaninm/pilot-shortlink/backend-b/internal/db"
)

// Image matches the Postgres used by scripts/verify.py.
const Image = "postgres:18.1-alpine"

// New returns a pool on a fresh, migrated database. The container stops when the test ends.
func New(t *testing.T) *pgxpool.Pool {
	t.Helper()
	ctx := context.Background()
	ctr, err := postgres.Run(ctx, Image, postgres.BasicWaitStrategies())
	testcontainers.CleanupContainer(t, ctr)
	if err != nil {
		t.Fatalf("start postgres (is Docker running?): %v", err)
	}
	dsn, err := ctr.ConnectionString(ctx, "sslmode=disable")
	if err != nil {
		t.Fatal(err)
	}
	pool, err := pgxpool.New(ctx, dsn)
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(pool.Close)
	if err := db.Migrate(ctx, pool, "up"); err != nil {
		t.Fatalf("migrate up: %v", err)
	}
	return pool
}
