// Command migrate applies the goose migrations. Usage: migrate up|down|status. Env: DATABASE_URL.
package main

import (
	"context"
	"fmt"
	"os"

	"github.com/jackc/pgx/v5/pgxpool"
	"github.com/tawaninm/pilot-shortlink/backend-b/internal/db"
)

func main() {
	if len(os.Args) != 2 {
		fmt.Fprintln(os.Stderr, "usage: migrate up|down|status")
		os.Exit(2)
	}
	ctx := context.Background()
	pool, err := pgxpool.New(ctx, os.Getenv("DATABASE_URL"))
	if err == nil {
		defer pool.Close()
		err = db.Migrate(ctx, pool, os.Args[1])
	}
	if err != nil {
		fmt.Fprintln(os.Stderr, "migrate:", err)
		os.Exit(1)
	}
}
