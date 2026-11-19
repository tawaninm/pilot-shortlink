package api

import (
	"context"
	"crypto/subtle"
	"errors"
	"net"
	"net/http"
	"strings"
	"time"

	"github.com/go-chi/chi/v5"
	"github.com/jackc/pgx/v5"
	"github.com/jackc/pgx/v5/pgtype"
	"github.com/tawaninm/pilot-shortlink/backend-b/internal/auth"
	"github.com/tawaninm/pilot-shortlink/backend-b/internal/db/gen"
	"github.com/tawaninm/pilot-shortlink/backend-b/internal/oauth"
)

const (
	oauthCookie          = "__Host-oauth"
	oauthErrorLocation   = "/login?error=oauth"
	oauthStateTTL        = 10 * time.Minute
	oauthRateWindow      = 10 * time.Minute
	oauthRateMax         = 30
	oauthOutboundTimeout = 10 * time.Second
)

type configResponse struct {
	TurnstileSiteKey *string `json:"turnstileSiteKey"`
}

func (s *server) config(w http.ResponseWriter, _ *http.Request) {
	writeJSON(w, http.StatusOK, configResponse{TurnstileSiteKey: s.TurnstileSiteKey})
}

func remoteAddrIP(remoteAddr string) string {
	trimmed := strings.TrimSpace(remoteAddr)
	if host, _, err := net.SplitHostPort(trimmed); err == nil && host != "" {
		return host
	}
	return trimmed
}

// clientIP resolves the client IP using TrustedProxyHops:
//   - <= 0: ignore X-Forwarded-For and return RemoteAddr (host part).
//   - N >= 1: take the Nth comma-separated X-Forwarded-For entry from the right;
//     must parse with net.ParseIP, otherwise fall back to RemoteAddr.
func (s *server) clientIP(r *http.Request) string {
	if s.TrustedProxyHops > 0 {
		if xff := strings.Join(r.Header.Values("X-Forwarded-For"), ","); xff != "" {
			parts := strings.Split(xff, ",")
			if len(parts) >= s.TrustedProxyHops {
				candidate := strings.TrimSpace(parts[len(parts)-s.TrustedProxyHops])
				if parsed := net.ParseIP(candidate); parsed != nil {
					return parsed.String()
				}
			}
		}
	}
	return remoteAddrIP(r.RemoteAddr)
}

func clearOAuthCookie(w http.ResponseWriter) {
	http.SetCookie(w, &http.Cookie{
		Name:     oauthCookie,
		Value:    "",
		Path:     "/",
		MaxAge:   -1,
		HttpOnly: true,
		Secure:   true,
		SameSite: http.SameSiteLaxMode,
	})
}

func (s *server) redirectOAuthError(w http.ResponseWriter, provider oauth.Provider, reason string, clearCookie bool) {
	// One log line per OAuth failure with provider and short static reason only (I7).
	s.Log.Error("oauth_failed", "provider", string(provider), "reason", reason)
	if clearCookie {
		clearOAuthCookie(w)
	}
	w.Header().Set("Location", oauthErrorLocation)
	w.WriteHeader(http.StatusFound)
}

func (s *server) oauthStart(w http.ResponseWriter, r *http.Request) {
	provider, ok := oauth.ParseProvider(chi.URLParam(r, "provider"))
	if !ok {
		writeProblem(w, http.StatusNotFound, "not_found", "")
		return
	}
	w.Header().Set("Cache-Control", "no-store")

	ctx := r.Context()
	now := s.Now()

	// Rate limit per client IP before writing state rows.
	n, err := s.q.BumpRate(ctx, gen.BumpRateParams{
		Key:         "oauth_start:" + s.clientIP(r),
		WindowStart: now.Truncate(oauthRateWindow),
	})
	if err != nil {
		s.redirectOAuthError(w, provider, "db_error", false)
		return
	}
	if n == 1 {
		_ = s.q.DeleteOldRates(ctx, now.Add(-time.Hour))
	}
	if n > oauthRateMax {
		s.redirectOAuthError(w, provider, "rate_limited", false)
		return
	}

	prov := s.OAuth.Get(provider)
	if prov == nil {
		s.redirectOAuthError(w, provider, "missing_credentials", false)
		return
	}

	state, verifier, challenge, err := oauth.NewStateAndVerifier()
	if err != nil {
		s.redirectOAuthError(w, provider, "crypto_error", false)
		return
	}

	if err := s.q.DeleteExpiredOAuthStates(ctx, now); err != nil {
		s.redirectOAuthError(w, provider, "db_error", false)
		return
	}
	if err := s.q.CreateOAuthState(ctx, gen.CreateOAuthStateParams{
		StateHash: auth.HashToken(state),
		Provider:  string(provider),
		Verifier:  verifier,
		ExpiresAt: now.Add(oauthStateTTL),
	}); err != nil {
		s.redirectOAuthError(w, provider, "db_error", false)
		return
	}

	redirectURI := oauth.RedirectURI(s.AppOrigin, provider)
	authURL, err := prov.BuildAuthorizeURL(redirectURI, state, challenge)
	if err != nil {
		s.redirectOAuthError(w, provider, "config_error", false)
		return
	}

	http.SetCookie(w, &http.Cookie{
		Name:     oauthCookie,
		Value:    state,
		Path:     "/",
		MaxAge:   int(oauthStateTTL.Seconds()),
		HttpOnly: true,
		Secure:   true,
		SameSite: http.SameSiteLaxMode,
	})
	w.Header().Set("Location", authURL)
	w.WriteHeader(http.StatusFound)
}

func (s *server) oauthCallback(w http.ResponseWriter, r *http.Request) {
	provider, ok := oauth.ParseProvider(chi.URLParam(r, "provider"))
	if !ok {
		writeProblem(w, http.StatusNotFound, "not_found", "")
		return
	}
	w.Header().Set("Cache-Control", "no-store")

	ctx := r.Context()
	now := s.Now()

	n, err := s.q.BumpRate(ctx, gen.BumpRateParams{
		Key:         "oauth_cb:" + s.clientIP(r),
		WindowStart: now.Truncate(oauthRateWindow),
	})
	if err != nil {
		s.redirectOAuthError(w, provider, "db_error", true)
		return
	}
	if n > oauthRateMax {
		s.redirectOAuthError(w, provider, "rate_limited", true)
		return
	}

	q := r.URL.Query()
	if q.Get("error") != "" {
		s.redirectOAuthError(w, provider, "provider_error", true)
		return
	}
	code, state := q.Get("code"), q.Get("state")
	if code == "" || state == "" || len(code) > 2048 || len(state) > 128 {
		s.redirectOAuthError(w, provider, "missing_params", true)
		return
	}

	c, err := r.Cookie(oauthCookie)
	if err != nil || c.Value == "" || subtle.ConstantTimeCompare([]byte(c.Value), []byte(state)) != 1 {
		s.redirectOAuthError(w, provider, "state_mismatch", true)
		return
	}

	prov := s.OAuth.Get(provider)
	if prov == nil {
		s.redirectOAuthError(w, provider, "missing_credentials", true)
		return
	}

	// Single-statement atomic consume enforces single-use and 10-minute expiry (I3, AC-O5).
	verifier, err := s.q.ConsumeOAuthState(ctx, gen.ConsumeOAuthStateParams{
		StateHash: auth.HashToken(state),
		Provider:  string(provider),
		ExpiresAt: now,
	})
	if errors.Is(err, pgx.ErrNoRows) {
		s.redirectOAuthError(w, provider, "invalid_state", true)
		return
	}
	if err != nil {
		s.redirectOAuthError(w, provider, "db_error", true)
		return
	}

	redirectURI := oauth.RedirectURI(s.AppOrigin, provider)
	outCtx, cancel := context.WithTimeout(ctx, oauthOutboundTimeout)
	defer cancel()

	accessToken, err := prov.Exchange(outCtx, code, redirectURI, verifier)
	if err != nil {
		s.redirectOAuthError(w, provider, "token_exchange_failed", true)
		return
	}

	identity, err := prov.Identity(outCtx, accessToken)
	if err != nil || identity.Subject == "" {
		s.redirectOAuthError(w, provider, "identity_fetch_failed", true)
		return
	}

	userID, reason := s.resolveOAuthUser(ctx, provider, identity)
	if reason != "" {
		s.redirectOAuthError(w, provider, reason, true)
		return
	}

	if err := s.setSession(w, r, userID); err != nil {
		s.redirectOAuthError(w, provider, "session_error", true)
		return
	}

	clearOAuthCookie(w)
	// Fixed destination "/" regardless of any return_to/next query parameters (I4, AC-O7).
	w.Header().Set("Location", "/")
	w.WriteHeader(http.StatusFound)
}

// resolveOAuthUser links or creates the local user inside one transaction (AC-O1..O4, AC-O9, AC-O12, I1, I2, I10).
func (s *server) resolveOAuthUser(ctx context.Context, provider oauth.Provider, id oauth.Identity) (int64, string) {
	tx, err := s.Pool.Begin(ctx)
	if err != nil {
		return 0, "db_error"
	}
	defer func() { _ = tx.Rollback(ctx) }()
	qtx := s.q.WithTx(tx)

	// a) Already linked (provider, subject) -> log into that user even if provider email changed (I2, AC-O9).
	uid, err := qtx.GetOAuthAccountUserID(ctx, gen.GetOAuthAccountUserIDParams{
		Provider: string(provider),
		Subject:  id.Subject,
	})
	if err == nil {
		if err := tx.Commit(ctx); err != nil {
			return 0, "db_error"
		}
		return uid, ""
	}
	if !errors.Is(err, pgx.ErrNoRows) {
		return 0, "db_error"
	}

	// b) Unverified provider email cannot link or create an account (I1, AC-O4).
	email, ok := parseEmail(id.Email)
	if !id.EmailVerified || !ok {
		return 0, "email_unverified"
	}

	now := pgtype.Timestamptz{Time: s.Now(), Valid: true}

	linkExisting := func(user gen.GetUserByEmailForUpdateRow) string {
		rows, err := qtx.InsertOAuthAccount(ctx, gen.InsertOAuthAccountParams{
			Provider: string(provider),
			Subject:  id.Subject,
			UserID:   user.ID,
		})
		if err != nil {
			return "db_error"
		}
		if rows == 0 {
			// Concurrent callback already linked (provider, subject); do not claim/wipe this user.
			return ""
		}
		if !user.EmailVerified {
			// Pre-hijack defense (I10, AC-O12): wipe squatter's password, sessions, and unused email tokens.
			if err := qtx.ClaimUnverifiedUser(ctx, gen.ClaimUnverifiedUserParams{
				ID:              user.ID,
				PasswordHash:    auth.OAuthOnlyPasswordHash,
				EmailVerifiedAt: now,
			}); err != nil {
				return "db_error"
			}
			if err := qtx.DeleteUserSessions(ctx, user.ID); err != nil {
				return "db_error"
			}
			if err := qtx.DeleteAllUnusedEmailTokens(ctx, user.ID); err != nil {
				return "db_error"
			}
		}
		return ""
	}

	// c) Existing user with matching lowercase email -> lock and link.
	existing, err := qtx.GetUserByEmailForUpdate(ctx, email)
	switch {
	case err == nil:
		if reason := linkExisting(existing); reason != "" {
			return 0, reason
		}
	case errors.Is(err, pgx.ErrNoRows):
		// d) Create verified OAuth-only user; on concurrent unique conflict, re-read locked row and link.
		newID, err := qtx.CreateVerifiedOAuthUserIfAbsent(ctx, gen.CreateVerifiedOAuthUserIfAbsentParams{
			Email:           email,
			PasswordHash:    auth.OAuthOnlyPasswordHash,
			EmailVerifiedAt: now,
		})
		switch {
		case err == nil:
			rows, err := qtx.InsertOAuthAccount(ctx, gen.InsertOAuthAccountParams{
				Provider: string(provider),
				Subject:  id.Subject,
				UserID:   newID,
			})
			if err != nil {
				return 0, "db_error"
			}
			if rows == 0 {
				// Concurrent callback already linked (provider, subject); discard unused new user and return winning user_id.
				uid, err = qtx.GetOAuthAccountUserID(ctx, gen.GetOAuthAccountUserIDParams{
					Provider: string(provider),
					Subject:  id.Subject,
				})
				if err != nil {
					return 0, "db_error"
				}
				return uid, ""
			}
		case errors.Is(err, pgx.ErrNoRows):
			existing, err := qtx.GetUserByEmailForUpdate(ctx, email)
			if err != nil {
				return 0, "db_error"
			}
			if reason := linkExisting(existing); reason != "" {
				return 0, reason
			}
		default:
			return 0, "db_error"
		}
	default:
		return 0, "db_error"
	}

	// Re-read oauth_accounts so concurrent inserts agree on the winning user_id.
	uid, err = qtx.GetOAuthAccountUserID(ctx, gen.GetOAuthAccountUserIDParams{
		Provider: string(provider),
		Subject:  id.Subject,
	})
	if err != nil {
		return 0, "db_error"
	}
	if err := tx.Commit(ctx); err != nil {
		return 0, "db_error"
	}
	return uid, ""
}
