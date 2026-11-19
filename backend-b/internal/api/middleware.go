package api

import (
	"context"
	"errors"
	"net/http"

	"github.com/jackc/pgx/v5"
	"github.com/tawaninm/pilot-shortlink/backend-b/internal/auth"
)

type ctxKey struct{}

// sessionUser is the logged-in user attached to the request by requireSession.
type sessionUser struct {
	ID            int64
	Email         string
	EmailVerified bool
}

func userFrom(r *http.Request) sessionUser { return r.Context().Value(ctxKey{}).(sessionUser) }

// requireOrigin rejects POST, PATCH and DELETE whose Origin is missing or not the app's (CSRF, A01).
func (s *server) requireOrigin(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		switch r.Method {
		case http.MethodPost, http.MethodPatch, http.MethodDelete, http.MethodPut:
			if r.Header.Get("Origin") != s.AppOrigin {
				writeProblem(w, http.StatusForbidden, "forbidden_origin", "")
				return
			}
		}
		next.ServeHTTP(w, r)
	})
}

// requireSession answers 401 unless the session cookie maps to an unexpired session.
func (s *server) requireSession(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Cache-Control", "no-store") // per-user data must not be cached (S7 security review)
		c, err := r.Cookie(sessionCookie)
		if err != nil {
			writeProblem(w, http.StatusUnauthorized, "unauthorized", "")
			return
		}
		u, err := s.q.GetSessionUser(r.Context(), auth.HashToken(c.Value))
		if errors.Is(err, pgx.ErrNoRows) {
			writeProblem(w, http.StatusUnauthorized, "unauthorized", "")
			return
		}
		if err != nil {
			s.fail(w, err)
			return
		}
		ctx := context.WithValue(r.Context(), ctxKey{}, sessionUser{ID: u.ID, Email: u.Email, EmailVerified: u.EmailVerified})
		next.ServeHTTP(w, r.WithContext(ctx))
	})
}

// requireVerified answers 403 email_unverified for unverified users on link-mutating routes (AC-V4).
func (s *server) requireVerified(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if !userFrom(r).EmailVerified {
			writeProblem(w, http.StatusForbidden, "email_unverified", "")
			return
		}
		next.ServeHTTP(w, r)
	})
}

// fail logs the real error and answers a generic 500 (A10: no internals to the client).
func (s *server) fail(w http.ResponseWriter, err error) {
	s.Log.Error("internal_error", "error", err.Error())
	writeProblem(w, http.StatusInternalServerError, "internal_error", "")
}
