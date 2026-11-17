// Package api is the HTTP layer of backend B. Spec: api/openapi.yaml, docs/architecture.md.
package api

import (
	"log/slog"
	"net/http"
	"strings"
	"time"

	"github.com/go-chi/chi/v5"
	"github.com/jackc/pgx/v5/pgxpool"
)

// Config is everything the HTTP layer needs from the outside.
type Config struct {
	Pool      *pgxpool.Pool
	AppOrigin string // e.g. https://pilot-shortlink-b.onrender.com, used for the Origin check and self_link
	Log       *slog.Logger
	Now       func() time.Time // injectable clock for tests; nil means time.Now
}

type server struct {
	Config
}

// NewHandler builds the router with all middleware.
func NewHandler(cfg Config) http.Handler {
	if cfg.Now == nil {
		cfg.Now = time.Now
	}
	s := &server{cfg}
	r := chi.NewRouter()
	r.Use(s.logRequests, s.recoverPanics)

	r.Get("/health", func(w http.ResponseWriter, _ *http.Request) {
		writeJSON(w, http.StatusOK, map[string]string{"status": "ok"})
	})

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
