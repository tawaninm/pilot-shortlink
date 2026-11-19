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
	"github.com/tawaninm/pilot-shortlink/backend-b/internal/captcha"
	"github.com/tawaninm/pilot-shortlink/backend-b/internal/mail"
	"github.com/tawaninm/pilot-shortlink/backend-b/internal/oauth"
)

func main() {
	log := slog.New(slog.NewJSONHandler(os.Stdout, nil))
	if err := run(log); err != nil {
		log.Error("server stopped", "error", err)
		os.Exit(1)
	}
}

func checkRenderGuards() error {
	if os.Getenv("RENDER") == "" {
		return nil
	}
	// Test-only modes expose tokens or bypass external checks; Render always sets RENDER, so refuse them there (I6).
	if os.Getenv("MAIL_MODE") == "capture" {
		return errors.New("MAIL_MODE=capture is not allowed on Render")
	}
	if os.Getenv("OAUTH_MODE") == "fake" {
		return errors.New("OAUTH_MODE=fake is not allowed on Render")
	}
	if os.Getenv("TURNSTILE_MODE") == "fake" {
		return errors.New("TURNSTILE_MODE=fake is not allowed on Render")
	}
	return nil
}

func run(log *slog.Logger) error {
	dbURL, origin := os.Getenv("DATABASE_URL"), os.Getenv("APP_ORIGIN")
	if dbURL == "" || origin == "" {
		return errors.New("DATABASE_URL and APP_ORIGIN are required")
	}
	if err := checkRenderGuards(); err != nil {
		return err
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
	oauthProviders := oauth.FromEnv(origin)
	captchaVerifier, turnstileSiteKey := captcha.FromEnv(origin)
	srv := &http.Server{
		Addr: ":" + port,
		Handler: api.NewHandler(api.Config{
			Pool:             pool,
			AppOrigin:        origin,
			Log:              log,
			Mailer:           mailer,
			Capture:          capture,
			OAuth:            oauthProviders,
			Captcha:          captchaVerifier,
			TurnstileSiteKey: turnstileSiteKey,
		}),
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
