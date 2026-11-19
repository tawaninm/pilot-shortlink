// Package api is the HTTP layer of backend B. Spec: api/openapi.yaml, docs/architecture.md.
package api

import (
	"log/slog"
	"net/http"
	"strings"
	"time"

	"github.com/go-chi/chi/v5"
	"github.com/jackc/pgx/v5/pgxpool"
	"github.com/tawaninm/pilot-shortlink/backend-b/internal/captcha"
	"github.com/tawaninm/pilot-shortlink/backend-b/internal/db/gen"
	"github.com/tawaninm/pilot-shortlink/backend-b/internal/mail"
	"github.com/tawaninm/pilot-shortlink/backend-b/internal/oauth"
	"github.com/tawaninm/pilot-shortlink/backend-b/internal/site"
)

// Config is everything the HTTP layer needs from the outside.
type Config struct {
	Pool             *pgxpool.Pool
	AppOrigin        string // e.g. https://pilot-shortlink-b.onrender.com, used for the Origin check and self_link
	Log              *slog.Logger
	Now              func() time.Time // injectable clock for tests; nil means time.Now
	Mailer           mail.Mailer      // outbound email port
	Capture          *mail.Capture    // non-nil only when MAIL_MODE == "capture"; registers GET /api/test/mail
	OAuth            *oauth.Providers // configured Google/GitHub providers
	Captcha          captcha.Verifier // Turnstile verifier; nil fails closed
	TurnstileSiteKey *string          // public Turnstile site key for GET /api/config; nil in fake mode
}

type server struct {
	Config
	q *gen.Queries
}

// NewHandler builds the router with all middleware.
func NewHandler(cfg Config) http.Handler {
	if cfg.Now == nil {
		cfg.Now = time.Now
	}
	if cfg.Log == nil {
		cfg.Log = slog.Default()
	}
	if cfg.Capture == nil {
		if c, ok := cfg.Mailer.(*mail.Capture); ok {
			cfg.Capture = c
		}
	}
	if cfg.Mailer == nil && cfg.Capture != nil {
		cfg.Mailer = cfg.Capture
	}
	if cfg.Captcha == nil {
		cfg.Captcha = captcha.NewCloudflare("")
	}
	s := &server{Config: cfg, q: gen.New(cfg.Pool)}
	r := chi.NewRouter()
	r.Use(s.logRequests, s.recoverPanics, noSniff)

	// The web app on the same origin (ADR-001). Static routes win over /{code} in chi, and these
	// names are reserved aliases, so they can never collide with a short link.
	app := site.New()
	r.Get("/", app.Index)
	r.Get("/login", app.Index)
	r.Get("/register", app.Index)
	r.Get("/verify-email", app.Index)
	r.Get("/forgot-password", app.Index)
	r.Get("/reset-password", app.Index)
	r.Handle("/assets/*", app.Assets())

	r.Get("/health", func(w http.ResponseWriter, _ *http.Request) {
		writeJSON(w, http.StatusOK, map[string]string{"status": "ok"})
	})

	r.Route("/api", func(r chi.Router) {
		r.Use(s.requireOrigin)
		r.Get("/config", s.config)
		r.Post("/auth/register", s.register)
		r.Post("/auth/login", s.login)
		r.Post("/auth/logout", s.logout)
		r.Post("/auth/verify-email", s.verifyEmail)
		r.Post("/auth/password/forgot", s.forgotPassword)
		r.Post("/auth/password/reset", s.resetPassword)
		r.Get("/auth/oauth/{provider}/start", s.oauthStart)
		r.Get("/auth/oauth/{provider}/callback", s.oauthCallback)
		if s.Capture != nil {
			r.Get("/test/mail", s.testMail)
		}
		r.Group(func(r chi.Router) {
			r.Use(s.requireSession)
			r.Get("/me", s.me)
			r.Post("/auth/verify-email/resend", s.resendVerifyEmail)
			r.Get("/links", s.listLinks)
			r.Get("/links/{code}/stats", s.linkStats)
			r.Group(func(r chi.Router) {
				r.Use(s.requireVerified)
				r.Post("/links", s.createLink)
				r.Patch("/links/{code}", s.patchLink)
				r.Delete("/links/{code}", s.deleteLink)
			})
		})
		r.NotFound(func(w http.ResponseWriter, _ *http.Request) { writeProblem(w, http.StatusNotFound, "not_found", "") })
	})

	r.Get("/{code}", s.redirect)

	r.NotFound(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path == "/api" || strings.HasPrefix(r.URL.Path, "/api/") {
			writeProblem(w, http.StatusNotFound, "not_found", "")
			return
		}
		writeNotFoundPage(w)
	})
	r.MethodNotAllowed(func(w http.ResponseWriter, _ *http.Request) {
		writeProblem(w, http.StatusMethodNotAllowed, "not_found", "")
	})
	return r
}

// noSniff stops browsers from guessing content types on every response (OWASP A02).
func noSniff(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("X-Content-Type-Options", "nosniff")
		next.ServeHTTP(w, r)
	})
}

// statusRecorder captures the status code for the request log.
type statusRecorder struct {
	http.ResponseWriter
	status int
}

func (r *statusRecorder) WriteHeader(code int) {
	r.status = code
	r.ResponseWriter.WriteHeader(code)
}

// logRequests writes one JSON line per request. It logs the chi route pattern (/api/links/{code}),
// never the raw path, so codes and emails do not reach the logs (docs/architecture.md section 8).
func (s *server) logRequests(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		start := s.Now()
		rec := &statusRecorder{ResponseWriter: w, status: http.StatusOK}
		next.ServeHTTP(rec, r)
		pattern := chi.RouteContext(r.Context()).RoutePattern()
		if pattern == "" || pattern == "/*" {
			pattern = "unmatched"
		}
		s.Log.Info("request", "method", r.Method, "path", pattern, "status", rec.status,
			"ms", s.Now().Sub(start).Milliseconds())
	})
}

// recoverPanics turns a panic into a 500 problem without leaking the panic text to the client.
func (s *server) recoverPanics(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		defer func() {
			if v := recover(); v != nil {
				s.Log.Error("panic", "error", v)
				writeProblem(w, http.StatusInternalServerError, "internal_error", "")
			}
		}()
		next.ServeHTTP(w, r)
	})
}
