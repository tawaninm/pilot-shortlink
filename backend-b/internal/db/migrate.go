// Package db holds the Postgres schema (goose migrations) and the sqlc-generated queries.
package db

import (
	"context"
	"database/sql"
	"embed"
	"fmt"
	"io/fs"

	"github.com/jackc/pgx/v5/pgxpool"
	"github.com/jackc/pgx/v5/stdlib"
	"github.com/pressly/goose/v3"
)

//go:embed migrations/*.sql
var migrations embed.FS

// Migrate runs goose with the given command ("up", "down", "status") against the pool's database.
func Migrate(ctx context.Context, pool *pgxpool.Pool, command string) error {
	conn := stdlib.OpenDBFromPool(pool)
	defer conn.Close()
	return migrate(ctx, conn, command)
}

func migrate(ctx context.Context, conn *sql.DB, command string) error {
	dir, err := fs.Sub(migrations, "migrations")
	if err != nil {
		return err
	}
	provider, err := goose.NewProvider(goose.DialectPostgres, conn, dir)
	if err != nil {
		return fmt.Errorf("goose provider: %w", err)
	}
	switch command {
	case "up":
		_, err = provider.Up(ctx)
	case "down":
		_, err = provider.Down(ctx)
	case "status":
		var st []*goose.MigrationStatus
		st, err = provider.Status(ctx)
		for _, s := range st {
			fmt.Printf("%-8s %s\n", s.State, s.Source.Path)
		}
	default:
		return fmt.Errorf("unknown migrate command %q (use up, down or status)", command)
	}
	return err
}
