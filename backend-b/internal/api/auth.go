package api

import (
	"encoding/json"
	"errors"
	"io"
	"net/http"
	"net/mail"
	"strings"
	"time"
	"unicode/utf8"

	"github.com/jackc/pgx/v5"
	"github.com/jackc/pgx/v5/pgconn"
	"github.com/tawaninm/pilot-shortlink/backend-b/internal/auth"
	"github.com/tawaninm/pilot-shortlink/backend-b/internal/db/gen"
)

const (
	sessionCookie  = "session"
	sessionTTL     = 7 * 24 * time.Hour
	loginWindow    = 15 * time.Minute
	loginMaxFailed = 5
	maxBodyBytes   = 16 << 10
)

// errBadBody means the body was not one valid JSON object of the expected shape.
var errBadBody = errors.New("invalid body")

// readJSON decodes exactly one JSON object, rejecting unknown fields (no mass assignment, A08).
func readJSON(w http.ResponseWriter, r *http.Request, dst any) error {
	dec := json.NewDecoder(http.MaxBytesReader(w, r.Body, maxBodyBytes))
	dec.DisallowUnknownFields()
	if err := dec.Decode(dst); err != nil {
		return errBadBody
	}
	if _, err := dec.Token(); !errors.Is(err, io.EOF) {
		return errBadBody
	}
	return nil
}

type credentials struct {
	Email    *string `json:"email"`
	Password *string `json:"password"`
}

// parse validates against Credentials in api/openapi.yaml and returns the lower-cased email.
func (c credentials) parse(w http.ResponseWriter) (email, password string, ok bool) {
	if c.Email == nil || c.Password == nil {
		writeProblem(w, http.StatusUnprocessableEntity, "invalid_body", "")
		return "", "", false
	}
	email = strings.ToLower(strings.TrimSpace(*c.Email))
	if addr, err := mail.ParseAddress(email); err != nil || addr.Address != email || len(email) > 254 {
		writeProblem(w, http.StatusUnprocessableEntity, "invalid_body", "email")
		return "", "", false
	}
	return email, *c.Password, true
}

func (s *server) setSession(w http.ResponseWriter, r *http.Request, userID int64) error {
	token, hash, err := auth.NewSessionToken()
	if err != nil {
		return err
	}
	err = s.q.CreateSession(r.Context(), gen.CreateSessionParams{TokenHash: hash, UserID: userID, ExpiresAt: s.Now().Add(sessionTTL)})
	if err != nil {
		return err
	}
	http.SetCookie(w, &http.Cookie{
		Name: sessionCookie, Value: token, Path: "/", MaxAge: int(sessionTTL.Seconds()),
		HttpOnly: true, Secure: true, SameSite: http.SameSiteLaxMode,
	})
	return nil
}

// hashTimed runs fn and logs how long Argon2 took (ADR-002 asks to watch hash cost).
func (s *server) hashTimed(op string, fn func()) {
	start := time.Now()
	fn()
	s.Log.Info("argon2", "op", op, "ms", time.Since(start).Milliseconds())
}

func (s *server) register(w http.ResponseWriter, r *http.Request) {
	var c credentials
	if readJSON(w, r, &c) != nil {
		writeProblem(w, http.StatusUnprocessableEntity, "invalid_body", "")
		return
	}
	email, password, ok := c.parse(w)
	if !ok {
		return
	}
	if n := utf8.RuneCountInString(password); n < 10 || n > 128 {
		writeProblem(w, http.StatusUnprocessableEntity, "weak_password", "password")
		return
	}
	var hash string
	var err error
	s.hashTimed("register", func() { hash, err = auth.HashPassword(password) })
	if err != nil {
		s.fail(w, err)
		return
	}
	id, err := s.q.CreateUser(r.Context(), gen.CreateUserParams{Email: email, PasswordHash: hash})
	if isUniqueViolation(err) {
		writeProblem(w, http.StatusConflict, "email_taken", "email")
		return
	}
	if err != nil {
		s.fail(w, err)
		return
	}
	if err := s.setSession(w, r, id); err != nil {
		s.fail(w, err)
		return
	}
	writeJSON(w, http.StatusCreated, map[string]string{"email": email})
}

func (s *server) login(w http.ResponseWriter, r *http.Request) {
	var c credentials
	if readJSON(w, r, &c) != nil {
		writeProblem(w, http.StatusUnprocessableEntity, "invalid_body", "")
		return
	}
	email, password, ok := c.parse(w)
	if !ok {
		return
	}
	ctx := r.Context()
	key, window := "login:"+email, s.Now().Truncate(loginWindow)

	failed, err := s.q.GetRateCount(ctx, gen.GetRateCountParams{Key: key, WindowStart: window})
	if err != nil && !errors.Is(err, pgx.ErrNoRows) {
		s.fail(w, err)
		return
	}
	if failed >= loginMaxFailed {
		writeRateLimited(w, int(window.Add(loginWindow).Sub(s.Now()).Seconds()))
		return
	}

	user, err := s.q.GetUserByEmail(ctx, email)
	var match bool
	switch {
	case errors.Is(err, pgx.ErrNoRows):
		s.hashTimed("login", func() { auth.BurnTime(password) })
	case err != nil:
		s.fail(w, err)
		return
	default:
		s.hashTimed("login", func() { match, err = auth.VerifyPassword(password, user.PasswordHash) })
		if err != nil {
			s.fail(w, err)
			return
		}
	}
	if !match {
		if _, err := s.q.BumpRate(ctx, gen.BumpRateParams{Key: key, WindowStart: window}); err != nil {
			s.fail(w, err)
			return
		}
		s.Log.Info("login_failed") // no email: logs must not identify people
		writeProblem(w, http.StatusUnauthorized, "unauthorized", "")
		return
	}
	if err := s.setSession(w, r, user.ID); err != nil {
		s.fail(w, err)
		return
	}
	writeJSON(w, http.StatusOK, map[string]string{"email": email})
}

func (s *server) logout(w http.ResponseWriter, r *http.Request) {
	if c, err := r.Cookie(sessionCookie); err == nil {
		if err := s.q.DeleteSession(r.Context(), auth.HashToken(c.Value)); err != nil {
			s.fail(w, err)
			return
		}
	}
	http.SetCookie(w, &http.Cookie{Name: sessionCookie, Value: "", Path: "/", MaxAge: -1, HttpOnly: true, Secure: true, SameSite: http.SameSiteLaxMode})
	w.WriteHeader(http.StatusNoContent)
}

func (s *server) me(w http.ResponseWriter, r *http.Request) {
	writeJSON(w, http.StatusOK, map[string]string{"email": userFrom(r).Email})
}

func isUniqueViolation(err error) bool {
	var pg *pgconn.PgError
	return errors.As(err, &pg) && pg.Code == "23505"
}
