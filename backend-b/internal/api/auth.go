package api

import (
	"context"
	"encoding/json"
	"errors"
	"io"
	"net/http"
	netmail "net/mail"
	"regexp"
	"strconv"
	"strings"
	"time"
	"unicode/utf8"

	"github.com/jackc/pgx/v5"
	"github.com/jackc/pgx/v5/pgconn"
	"github.com/jackc/pgx/v5/pgtype"
	"github.com/tawaninm/pilot-shortlink/backend-b/internal/auth"
	"github.com/tawaninm/pilot-shortlink/backend-b/internal/db/gen"
	"github.com/tawaninm/pilot-shortlink/backend-b/internal/mail"
)

const (
	sessionCookie   = "__Host-session" // __Host-: HTTPS only, Path=/, no Domain (S7 security review)
	sessionTTL      = 7 * 24 * time.Hour
	emailTokenTTL   = 30 * time.Minute
	mailSendTimeout = 10 * time.Second
	mailRateWindow  = time.Hour
	mailRateMax     = 3
	loginWindow     = 15 * time.Minute
	loginMaxFailed  = 5
	maxBodyBytes    = 16 << 10
)

var tokenPattern = regexp.MustCompile(`^[A-Za-z0-9_-]{43}$`)

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

type meResponse struct {
	Email         string `json:"email"`
	EmailVerified bool   `json:"emailVerified"`
}

func parseEmail(raw string) (string, bool) {
	email := strings.ToLower(strings.TrimSpace(raw))
	if addr, err := netmail.ParseAddress(email); err != nil || addr.Address != email || len(email) > 254 {
		return "", false
	}
	return email, true
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
	email, ok = parseEmail(*c.Email)
	if !ok {
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

// issueEmailToken replaces any unused token for (userID, purpose) with a fresh 32-byte token (stored as SHA-256 hex).
func (s *server) issueEmailToken(ctx context.Context, userID int64, purpose string) (token, hash string, err error) {
	token, hash, err = auth.NewSessionToken()
	if err != nil {
		return "", "", err
	}
	tx, err := s.Pool.Begin(ctx)
	if err != nil {
		return "", "", err
	}
	defer func() { _ = tx.Rollback(ctx) }()
	qtx := s.q.WithTx(tx)

	if _, err := qtx.LockUser(ctx, userID); err != nil {
		return "", "", err
	}
	if err := qtx.DeleteUnusedEmailTokens(ctx, gen.DeleteUnusedEmailTokensParams{UserID: userID, Purpose: purpose}); err != nil {
		return "", "", err
	}
	if err := qtx.CreateEmailToken(ctx, gen.CreateEmailTokenParams{
		TokenHash: hash,
		UserID:    userID,
		Purpose:   purpose,
		ExpiresAt: s.Now().Add(emailTokenTTL),
	}); err != nil {
		return "", "", err
	}
	if err := tx.Commit(ctx); err != nil {
		return "", "", err
	}
	return token, hash, nil
}

// sendMailAsync dispatches an email in the background with a 10 s timeout detached from request cancellation.
// Logs contain only the purpose — never the token, link, email or password (A09).
func (s *server) sendMailAsync(reqCtx context.Context, purpose string, msg mail.Mail) {
	if s.Mailer == nil {
		return
	}
	baseCtx := context.WithoutCancel(reqCtx)
	go func() {
		ctx, cancel := context.WithTimeout(baseCtx, mailSendTimeout)
		defer cancel()
		if err := s.Mailer.Send(ctx, msg); err != nil {
			s.Log.Error("mail_failed", "purpose", purpose, "error", err.Error()) // adapter errors carry only a status, never token, link or address
			return
		}
		s.Log.Info("mail_sent", "purpose", purpose)
	}()
}

// hashSlots caps concurrent Argon2 runs. Each takes about 19 MiB, so a burst of logins or sign-ups
// with random emails could exhaust memory on a small instance (S7 security review). Extra requests wait.
var hashSlots = make(chan struct{}, 4)

// hashTimed runs fn and logs how long Argon2 took (ADR-002 asks to watch hash cost).
func (s *server) hashTimed(op string, fn func()) {
	hashSlots <- struct{}{}
	defer func() { <-hashSlots }()
	start := time.Now()
	fn()
	s.Log.Info("argon2", "op", op, "ms", time.Since(start).Milliseconds())
}

type registerBody struct {
	Email    *string `json:"email"`
	Password *string `json:"password"`
	Captcha  *string `json:"captcha"`
}

func (s *server) verifyCaptcha(r *http.Request, token *string) bool {
	if token == nil || *token == "" || s.Captcha == nil || !s.Captcha.Verify(r.Context(), *token, clientIP(r)) {
		s.Log.Info("captcha_failed") // never log the token, IP, or email (I7)
		return false
	}
	return true
}

func (s *server) register(w http.ResponseWriter, r *http.Request) {
	var body registerBody
	if readJSON(w, r, &body) != nil || body.Email == nil || body.Password == nil {
		writeProblem(w, http.StatusUnprocessableEntity, "invalid_body", "")
		return
	}
	if !s.verifyCaptcha(r, body.Captcha) {
		writeProblem(w, http.StatusForbidden, "captcha_failed", "")
		return
	}
	email, ok := parseEmail(*body.Email)
	if !ok {
		writeProblem(w, http.StatusUnprocessableEntity, "invalid_body", "email")
		return
	}
	password := *body.Password
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
	if token, tokenHash, err := s.issueEmailToken(r.Context(), id, mail.PurposeVerify); err != nil {
		s.Log.Error("mail_failed", "purpose", mail.PurposeVerify)
	} else {
		s.sendMailAsync(r.Context(), mail.PurposeVerify, mail.Build(mail.PurposeVerify, email, s.AppOrigin, token, tokenHash))
	}
	writeJSON(w, http.StatusCreated, meResponse{Email: email, EmailVerified: false})
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

	// Count the attempt atomically before checking the password. Reading first and writing after a
	// failure let parallel guesses all pass the check (S7 security review). Success resets the count.
	attempts, err := s.q.BumpRate(ctx, gen.BumpRateParams{Key: key, WindowStart: window})
	if err != nil {
		s.fail(w, err)
		return
	}
	if attempts > loginMaxFailed {
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
		s.hashTimed("login", func() {
			match, err = auth.VerifyPassword(password, user.PasswordHash)
			if err != nil {
				// Non-PHC hash (e.g. "!oauth" on OAuth-only accounts): burn equal Argon2 time and fail with 401 (I8, AC-O8).
				auth.BurnTime(password)
				match = false
				err = nil
			}
		})
	}
	if !match {
		s.Log.Info("login_failed") // no email: logs must not identify people
		writeProblem(w, http.StatusUnauthorized, "unauthorized", "")
		return
	}
	if err := s.q.ResetRate(ctx, key); err != nil {
		s.fail(w, err)
		return
	}
	if err := s.setSession(w, r, user.ID); err != nil {
		s.fail(w, err)
		return
	}
	writeJSON(w, http.StatusOK, meResponse{Email: email, EmailVerified: user.EmailVerified})
}

func (s *server) logout(w http.ResponseWriter, r *http.Request) {
	w.Header().Set("Cache-Control", "no-store")
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
	u := userFrom(r)
	writeJSON(w, http.StatusOK, meResponse{Email: u.Email, EmailVerified: u.EmailVerified})
}

type tokenBody struct {
	Token *string `json:"token"`
}

func (s *server) verifyEmail(w http.ResponseWriter, r *http.Request) {
	var body tokenBody
	if readJSON(w, r, &body) != nil || body.Token == nil {
		writeProblem(w, http.StatusUnprocessableEntity, "invalid_body", "")
		return
	}
	if !tokenPattern.MatchString(*body.Token) {
		writeProblem(w, http.StatusUnprocessableEntity, "invalid_body", "token")
		return
	}
	ctx := r.Context()
	tx, err := s.Pool.Begin(ctx)
	if err != nil {
		s.fail(w, err)
		return
	}
	defer func() { _ = tx.Rollback(ctx) }()
	qtx := s.q.WithTx(tx)

	userID, err := qtx.ConsumeEmailToken(ctx, gen.ConsumeEmailTokenParams{
		TokenHash: auth.HashToken(*body.Token),
		Purpose:   mail.PurposeVerify,
		UsedAt:    pgtype.Timestamptz{Time: s.Now(), Valid: true},
	})
	if errors.Is(err, pgx.ErrNoRows) {
		writeProblem(w, http.StatusBadRequest, "token_invalid", "")
		return
	}
	if err != nil {
		s.fail(w, err)
		return
	}
	if err := qtx.MarkEmailVerified(ctx, userID); err != nil {
		s.fail(w, err)
		return
	}
	if err := tx.Commit(ctx); err != nil {
		s.fail(w, err)
		return
	}
	w.WriteHeader(http.StatusNoContent)
}

func (s *server) resendVerifyEmail(w http.ResponseWriter, r *http.Request) {
	user, ctx := userFrom(r), r.Context()
	if user.EmailVerified {
		w.WriteHeader(http.StatusNoContent)
		return
	}
	window := s.Now().Truncate(mailRateWindow)
	n, err := s.q.BumpRate(ctx, gen.BumpRateParams{
		Key:         "resend:" + strconv.FormatInt(user.ID, 10),
		WindowStart: window,
	})
	if err != nil {
		s.fail(w, err)
		return
	}
	if n > mailRateMax {
		writeRateLimited(w, int(window.Add(mailRateWindow).Sub(s.Now()).Seconds()))
		return
	}
	token, tokenHash, err := s.issueEmailToken(ctx, user.ID, mail.PurposeVerify)
	if err != nil {
		s.fail(w, err)
		return
	}
	s.sendMailAsync(ctx, mail.PurposeVerify, mail.Build(mail.PurposeVerify, user.Email, s.AppOrigin, token, tokenHash))
	w.WriteHeader(http.StatusNoContent)
}

type forgotBody struct {
	Email   *string `json:"email"`
	Captcha *string `json:"captcha"`
}

func (s *server) forgotPassword(w http.ResponseWriter, r *http.Request) {
	var body forgotBody
	if readJSON(w, r, &body) != nil || body.Email == nil {
		writeProblem(w, http.StatusUnprocessableEntity, "invalid_body", "")
		return
	}
	// Check captcha BEFORE email format and BEFORE per-email rate limit so bad captchas cannot burn the owner's quota (I9, AC-T4).
	if !s.verifyCaptcha(r, body.Captcha) {
		writeProblem(w, http.StatusForbidden, "captcha_failed", "")
		return
	}
	email, ok := parseEmail(*body.Email)
	if !ok {
		writeProblem(w, http.StatusUnprocessableEntity, "invalid_body", "email")
		return
	}
	ctx := r.Context()
	window := s.Now().Truncate(mailRateWindow)
	n, err := s.q.BumpRate(ctx, gen.BumpRateParams{
		Key:         "forgot:" + email,
		WindowStart: window,
	})
	if err != nil {
		s.fail(w, err)
		return
	}
	if n > mailRateMax {
		w.WriteHeader(http.StatusNoContent)
		return
	}
	user, err := s.q.GetUserByEmail(ctx, email)
	if errors.Is(err, pgx.ErrNoRows) {
		w.WriteHeader(http.StatusNoContent)
		return
	}
	if err != nil {
		s.fail(w, err)
		return
	}
	if token, tokenHash, err := s.issueEmailToken(ctx, user.ID, mail.PurposeReset); err != nil {
		s.Log.Error("mail_failed", "purpose", mail.PurposeReset)
	} else {
		s.sendMailAsync(ctx, mail.PurposeReset, mail.Build(mail.PurposeReset, email, s.AppOrigin, token, tokenHash))
	}
	w.WriteHeader(http.StatusNoContent)
}

type resetBody struct {
	Token    *string `json:"token"`
	Password *string `json:"password"`
}

func (s *server) resetPassword(w http.ResponseWriter, r *http.Request) {
	var body resetBody
	if readJSON(w, r, &body) != nil || body.Token == nil || body.Password == nil {
		writeProblem(w, http.StatusUnprocessableEntity, "invalid_body", "")
		return
	}
	if n := utf8.RuneCountInString(*body.Password); n < 10 || n > 128 {
		writeProblem(w, http.StatusUnprocessableEntity, "weak_password", "password")
		return
	}
	if !tokenPattern.MatchString(*body.Token) {
		writeProblem(w, http.StatusUnprocessableEntity, "invalid_body", "token")
		return
	}
	ctx := r.Context()
	tx, err := s.Pool.Begin(ctx)
	if err != nil {
		s.fail(w, err)
		return
	}
	defer func() { _ = tx.Rollback(ctx) }()
	qtx := s.q.WithTx(tx)

	userID, err := qtx.ConsumeEmailToken(ctx, gen.ConsumeEmailTokenParams{
		TokenHash: auth.HashToken(*body.Token),
		Purpose:   mail.PurposeReset,
		UsedAt:    pgtype.Timestamptz{Time: s.Now(), Valid: true},
	})
	if errors.Is(err, pgx.ErrNoRows) {
		writeProblem(w, http.StatusBadRequest, "token_invalid", "")
		return
	}
	if err != nil {
		s.fail(w, err)
		return
	}
	var hash string
	s.hashTimed("reset", func() { hash, err = auth.HashPassword(*body.Password) })
	if err != nil {
		s.fail(w, err)
		return
	}
	if err := qtx.ResetUserPassword(ctx, gen.ResetUserPasswordParams{ID: userID, PasswordHash: hash}); err != nil {
		s.fail(w, err)
		return
	}
	if err := qtx.DeleteUserSessions(ctx, userID); err != nil {
		s.fail(w, err)
		return
	}
	if err := tx.Commit(ctx); err != nil {
		s.fail(w, err)
		return
	}
	w.WriteHeader(http.StatusNoContent)
}

func (s *server) testMail(w http.ResponseWriter, r *http.Request) {
	to := strings.ToLower(strings.TrimSpace(r.URL.Query().Get("to")))
	writeJSON(w, http.StatusOK, map[string]any{"items": s.Capture.For(to)})
}

func isUniqueViolation(err error) bool {
	var pg *pgconn.PgError
	return errors.As(err, &pg) && pg.Code == "23505"
}
