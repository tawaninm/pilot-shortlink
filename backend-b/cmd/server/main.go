// Command server runs backend B. Env: DATABASE_URL, APP_ORIGIN, PORT (default 8080).
package main

import (
	"context"
	"errors"
	"log/slog"
	"net/http"
	"os"
	"os/signal"
	"syscall"
	"time"

	"github.com/jackc/pgx/v5/pgxpool"
	"github.com/tawaninm/pilot-shortlink/backend-b/internal/api"
	"github.com/tawaninm/pilot-shortlink/backend-b/internal/mail"
)

func main() {
	log := slog.New(slog.NewJSONHandler(os.Stdout, nil))
	if err := run(log); err != nil {
		log.Error("server stopped", "error", err)
		os.Exit(1)
	}
}

func run(log *slog.Logger) error {
	dbURL, origin := os.Getenv("DATABASE_URL"), os.Getenv("APP_ORIGIN")
	if dbURL == "" || origin == "" {
		return errors.New("DATABASE_URL and APP_ORIGIN are required")
	}
	// capture mode exposes every mailed token at GET /api/test/mail; Render always sets RENDER, so refuse it there.
	if os.Getenv("MAIL_MODE") == "capture" && os.Getenv("RENDER") != "" {
		return errors.New("MAIL_MODE=capture is not allowed on Render")
	}
	port := os.Getenv("PORT")
	if port == "" {
		port = "8080"
	}

	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()

	pool, err := pgxpool.New(ctx, dbURL)
	if err != nil {
		return err
	}
	defer pool.Close()

	mailer, capture := mail.FromEnv()
	srv := &http.Server{
		Addr:              ":" + port,
		Handler:           api.NewHandler(api.Config{Pool: pool, AppOrigin: origin, Log: log, Mailer: mailer, Capture: capture}),
		ReadHeaderTimeout: 5 * time.Second,
		ReadTimeout:       10 * time.Second,
		WriteTimeout:      15 * time.Second,
		IdleTimeout:       60 * time.Second,
	}
	errc := make(chan error, 1)
	go func() { errc <- srv.ListenAndServe() }()
	log.Info("listening", "port", port)

	select {
	case err := <-errc:
		return err
	case <-ctx.Done():
		shutdownCtx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
		defer cancel()
		return srv.Shutdown(shutdownCtx)
	}
}
