package api_test

import (
	"context"
	"encoding/json"
	"log/slog"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"testing"
	"time"

	"github.com/tawaninm/pilot-shortlink/backend-b/internal/api"
	"github.com/tawaninm/pilot-shortlink/backend-b/internal/auth"
	"github.com/tawaninm/pilot-shortlink/backend-b/internal/captcha"
	"github.com/tawaninm/pilot-shortlink/backend-b/internal/db/gen"
	"github.com/tawaninm/pilot-shortlink/backend-b/internal/mail"
	"github.com/tawaninm/pilot-shortlink/backend-b/internal/oauth"
	"github.com/tawaninm/pilot-shortlink/backend-b/internal/testdb"
)

func TestOAuthAndCaptcha(t *testing.T) {
	pool := testdb.New(t)
	localOrigin := "http://localhost:8080"

	t.Run("expired state is not consumed (injected clock) and logs leak no secrets", func(t *testing.T) {
		const (
			secretCode   = "secret-auth-code-999"
			secretToken  = "secret-access-token-888"
			secretEmail  = "oauth-expiry@example.test"
			secretBody   = "provider-secret-body-777"
		)
		var tokenHits int
		idp := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
			switch r.URL.Path {
			case "/google/token":
				tokenHits++
				_, _ = w.Write([]byte(`{"access_token":"` + secretToken + `"}`))
			case "/google/userinfo":
				_, _ = w.Write([]byte(`{"sub":"sub-exp-1","email":"` + secretEmail + `","email_verified":true,"note":"` + secretBody + `"}`))
			default:
				w.WriteHeader(http.StatusNotFound)
			}
		}))
		defer idp.Close()

		now := time.Date(2026, 11, 19, 12, 0, 0, 0, time.UTC)
		var logs syncBuffer
		h := api.NewHandler(api.Config{
			Pool:      pool,
			AppOrigin: localOrigin,
			Log:       slog.New(slog.NewJSONHandler(&logs, nil)),
			Now:       func() time.Time { return now },
			OAuth: oauth.Resolve(oauth.Env{
				AppOrigin: localOrigin,
				Mode:      "fake",
				FakeBase:  idp.URL,
			}),
			Captcha: captcha.Fake{},
		})

		startReq := httptest.NewRequest(http.MethodGet, "/api/auth/oauth/google/start", nil)
		startReq.RemoteAddr = "198.51.100.10:1234"
		startRec := httptest.NewRecorder()
		h.ServeHTTP(startRec, startReq)
		if startRec.Code != http.StatusFound {
			t.Fatalf("start status = %d, want 302", startRec.Code)
		}
		authURL, err := url.Parse(startRec.Header().Get("Location"))
		if err != nil {
			t.Fatal(err)
		}
		state := authURL.Query().Get("state")
		if state == "" {
			t.Fatal("start did not put state in Location")
		}
		var stateCookie *http.Cookie
		for _, c := range startRec.Result().Cookies() {
			if c.Name == "__Host-oauth" {
				stateCookie = c
			}
		}
		if stateCookie == nil || stateCookie.Value != state {
			t.Fatalf("missing or mismatched __Host-oauth cookie: %+v", stateCookie)
		}

		// Read stored verifier before expiry so we can verify logs never print it either.
		var verifier string
		if err := pool.QueryRow(context.Background(),
			`SELECT verifier FROM oauth_states WHERE state_hash = $1`, auth.HashToken(state)).Scan(&verifier); err != nil {
			t.Fatalf("state row missing: %v", err)
		}

		// Advance clock to the exact 10-minute expiry boundary -> state must NOT be consumed.
		now = now.Add(10 * time.Minute)

		cbReq := httptest.NewRequest(http.MethodGet, "/api/auth/oauth/google/callback?code="+secretCode+"&state="+state, nil)
		cbReq.RemoteAddr = "198.51.100.10:1235"
		cbReq.AddCookie(stateCookie)
		cbRec := httptest.NewRecorder()
		h.ServeHTTP(cbRec, cbReq)

		if cbRec.Code != http.StatusFound || cbRec.Header().Get("Location") != "/login?error=oauth" {
			t.Fatalf("expired callback = %d Location=%q, want 302 /login?error=oauth", cbRec.Code, cbRec.Header().Get("Location"))
		}
		if tokenHits != 0 {
			t.Fatalf("expired state triggered %d token exchange calls, want 0", tokenHits)
		}
		for _, c := range cbRec.Result().Cookies() {
			if c.Name == "__Host-session" {
				t.Fatal("expired state must not set session cookie")
			}
		}
		// Row still exists in oauth_states because DELETE ... WHERE expires_at > $3 did not consume it.
		var stillThere int
		if err := pool.QueryRow(context.Background(),
			`SELECT count(*) FROM oauth_states WHERE state_hash = $1`, auth.HashToken(state)).Scan(&stillThere); err != nil || stillThere != 1 {
			t.Fatalf("expired state row count = %d (err=%v), want 1 (unconsumed)", stillThere, err)
		}

		out := logs.String()
		if !strings.Contains(out, `"oauth_failed"`) || !strings.Contains(out, `"provider":"google"`) || !strings.Contains(out, `"reason":"invalid_state"`) {
			t.Fatalf("expected structured oauth_failed log, got:\n%s", out)
		}
		for _, secret := range []string{secretCode, state, verifier, secretToken, secretEmail, secretBody} {
			if strings.Contains(out, secret) {
				t.Fatalf("log leaked %q:\n%s", secret, out)
			}
		}
	})

	t.Run("missing provider credentials redirects to error page with one log line and never 500", func(t *testing.T) {
		var logs syncBuffer
		h := api.NewHandler(api.Config{
			Pool:      pool,
			AppOrigin: "https://pilot-shortlink-b.onrender.com",
			Log:       slog.New(slog.NewJSONHandler(&logs, nil)),
			OAuth:     oauth.Resolve(oauth.Env{AppOrigin: "https://pilot-shortlink-b.onrender.com"}),
			Captcha:   captcha.Fake{},
		})

		req := httptest.NewRequest(http.MethodGet, "/api/auth/oauth/google/start", nil)
		req.RemoteAddr = "198.51.100.11:1000"
		rec := httptest.NewRecorder()
		h.ServeHTTP(rec, req)

		if rec.Code != http.StatusFound || rec.Header().Get("Location") != "/login?error=oauth" {
			t.Fatalf("missing creds start = %d Location=%q, want 302 /login?error=oauth", rec.Code, rec.Header().Get("Location"))
		}
		out := logs.String()
		if strings.Count(out, `"oauth_failed"`) != 1 || !strings.Contains(out, `"reason":"missing_credentials"`) {
			t.Fatalf("expected single oauth_failed missing_credentials log line, got:\n%s", out)
		}
	})

	t.Run("OAuth-only account login returns 401 and pre-hijack wipes unused email tokens", func(t *testing.T) {
		const (
			email        = "prehijack@example.test"
			secretCode   = "ph-code-secret"
			secretToken  = "ph-access-secret"
			secretBody   = "ph-body-secret"
		)
		idp := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
			switch r.URL.Path {
			case "/google/token":
				_, _ = w.Write([]byte(`{"access_token":"` + secretToken + `"}`))
			case "/google/userinfo":
				_, _ = w.Write([]byte(`{"sub":"sub-ph-1","email":"` + email + `","email_verified":true,"extra":"` + secretBody + `"}`))
			}
		}))
		defer idp.Close()

		capMail := mail.NewCapture()
		var logs syncBuffer
		h := api.NewHandler(api.Config{
			Pool:      pool,
			AppOrigin: localOrigin,
			Log:       slog.New(slog.NewJSONHandler(&logs, nil)),
			Mailer:    capMail,
			Capture:   capMail,
			OAuth: oauth.Resolve(oauth.Env{
				AppOrigin: localOrigin,
				Mode:      "fake",
				FakeBase:  idp.URL,
			}),
			Captcha: captcha.Fake{},
		})

		// Squatter registers email (unverified) and receives a verify token.
		regReq := httptest.NewRequest(http.MethodPost, "/api/auth/register",
			strings.NewReader(`{"email":"`+email+`","password":"correct-horse-battery","captcha":"fake-pass"}`))
		regReq.Header.Set("Content-Type", "application/json")
		regReq.Header.Set("Origin", localOrigin)
		regRec := httptest.NewRecorder()
		h.ServeHTTP(regRec, regReq)
		if regRec.Code != http.StatusCreated {
			t.Fatalf("squatter register: %d %s", regRec.Code, regRec.Body.String())
		}
		verifyTok := waitCapturedToken(t, capMail, email, 1)

		// Real owner logs in with Google -> claims the unverified account, wipes password, sessions, and unused email tokens.
		startReq := httptest.NewRequest(http.MethodGet, "/api/auth/oauth/google/start", nil)
		startReq.RemoteAddr = "198.51.100.12:1000"
		startRec := httptest.NewRecorder()
		h.ServeHTTP(startRec, startReq)
		authURL, _ := url.Parse(startRec.Header().Get("Location"))
		state := authURL.Query().Get("state")
		var stateCookie *http.Cookie
		for _, c := range startRec.Result().Cookies() {
			if c.Name == "__Host-oauth" {
				stateCookie = c
			}
		}

		cbReq := httptest.NewRequest(http.MethodGet, "/api/auth/oauth/google/callback?code="+secretCode+"&state="+state, nil)
		cbReq.RemoteAddr = "198.51.100.12:1001"
		cbReq.AddCookie(stateCookie)
		cbRec := httptest.NewRecorder()
		h.ServeHTTP(cbRec, cbReq)
		if cbRec.Code != http.StatusFound || cbRec.Header().Get("Location") != "/" {
			t.Fatalf("callback = %d Location=%q, want 302 /", cbRec.Code, cbRec.Header().Get("Location"))
		}

		// Squatter's unused verify token was deleted (I10).
		vReq := httptest.NewRequest(http.MethodPost, "/api/auth/verify-email", strings.NewReader(`{"token":"`+verifyTok+`"}`))
		vReq.Header.Set("Content-Type", "application/json")
		vReq.Header.Set("Origin", localOrigin)
		vRec := httptest.NewRecorder()
		h.ServeHTTP(vRec, vReq)
		if vRec.Code != http.StatusBadRequest {
			t.Fatalf("squatter verify token after OAuth claim: %d, want 400", vRec.Code)
		}

		// Password login against "!oauth" hash returns 401 unauthorized (never 500, AC-O8).
		lReq := httptest.NewRequest(http.MethodPost, "/api/auth/login",
			strings.NewReader(`{"email":"`+email+`","password":"correct-horse-battery"}`))
		lReq.Header.Set("Content-Type", "application/json")
		lReq.Header.Set("Origin", localOrigin)
		lRec := httptest.NewRecorder()
		h.ServeHTTP(lRec, lReq)
		if lRec.Code != http.StatusUnauthorized {
			t.Fatalf("login against !oauth user = %d %s, want 401", lRec.Code, lRec.Body.String())
		}

		out := logs.String()
		for _, secret := range []string{secretCode, state, secretToken, secretBody, email} {
			if strings.Contains(out, secret) {
				t.Fatalf("OAuth log leaked %q:\n%s", secret, out)
			}
		}
	})

	t.Run("forgot password checks captcha before bumping per-email rate counter and never logs captcha token", func(t *testing.T) {
		const (
			email           = "forgot-order@example.test"
			badCaptchaToken = "secret-bad-captcha-token-xyz"
		)
		var logs syncBuffer
		capMail := mail.NewCapture()
		h := api.NewHandler(api.Config{
			Pool:      pool,
			AppOrigin: testOrigin,
			Log:       slog.New(slog.NewJSONHandler(&logs, nil)),
			Mailer:    capMail,
			Capture:   capMail,
			Captcha:   captcha.Fake{},
		})

		if rec := postJSON(t, h, "/api/auth/register",
			`{"email":"`+email+`","password":"correct-horse-battery","captcha":"fake-pass"}`); rec.Code != http.StatusCreated {
			t.Fatalf("register: %d %s", rec.Code, rec.Body.String())
		}
		waitCapturedToken(t, capMail, email, 1)

		// 5 failed captcha attempts must all return 403 and leave rate_limits untouched for forgot:<email>.
		for range 5 {
			rec := postJSON(t, h, "/api/auth/password/forgot", `{"email":"`+email+`","captcha":"`+badCaptchaToken+`"}`)
			if rec.Code != http.StatusForbidden || !strings.Contains(rec.Body.String(), `"captcha_failed"`) {
				t.Fatalf("bad captcha forgot = %d %s, want 403 captcha_failed", rec.Code, rec.Body.String())
			}
		}
		var rateRows int
		if err := pool.QueryRow(context.Background(),
			`SELECT count(*) FROM rate_limits WHERE key = $1`, "forgot:"+email).Scan(&rateRows); err != nil || rateRows != 0 {
			t.Fatalf("failed captchas bumped forgot rate_limits: rows=%d err=%v, want 0", rateRows, err)
		}

		// Valid captcha still gets its full quota and sends reset mail.
		rec := postJSON(t, h, "/api/auth/password/forgot", `{"email":"`+email+`","captcha":"fake-pass"}`)
		if rec.Code != http.StatusNoContent {
			t.Fatalf("valid captcha forgot = %d, want 204", rec.Code)
		}
		waitCapturedToken(t, capMail, email, 2)

		out := logs.String()
		for _, secret := range []string{badCaptchaToken, "fake-pass", email} {
			if strings.Contains(out, secret) {
				t.Fatalf("captcha log leaked %q:\n%s", secret, out)
			}
		}
	})

	t.Run("GET /api/config returns turnstileSiteKey or null", func(t *testing.T) {
		hNull := api.NewHandler(api.Config{AppOrigin: localOrigin, Captcha: captcha.Fake{}})
		rec := httptest.NewRecorder()
		hNull.ServeHTTP(rec, httptest.NewRequest(http.MethodGet, "/api/config", nil))
		if rec.Code != http.StatusOK || strings.TrimSpace(rec.Body.String()) != `{"turnstileSiteKey":null}` {
			t.Fatalf("config null = %d %s", rec.Code, rec.Body.String())
		}

		sk := "0x4AAAAAAAtest"
		hKey := api.NewHandler(api.Config{AppOrigin: testOrigin, TurnstileSiteKey: &sk})
		rec = httptest.NewRecorder()
		hKey.ServeHTTP(rec, httptest.NewRequest(http.MethodGet, "/api/config", nil))
		var body map[string]any
		if err := json.Unmarshal(rec.Body.Bytes(), &body); err != nil || rec.Code != http.StatusOK || body["turnstileSiteKey"] != sk {
			t.Fatalf("config key = %d %s (err=%v)", rec.Code, rec.Body.String(), err)
		}
	})

	t.Run("oauth start prunes stale rate_limits rows", func(t *testing.T) {
		now := time.Date(2026, 11, 19, 14, 0, 0, 0, time.UTC)
		staleWindow := now.Add(-2 * time.Hour)
		if _, err := pool.Exec(context.Background(),
			`INSERT INTO rate_limits (key, window_start, count) VALUES ('oauth_start:stale', $1, 5)`, staleWindow); err != nil {
			t.Fatal(err)
		}

		idp := httptest.NewServer(http.NotFoundHandler())
		defer idp.Close()

		h := api.NewHandler(api.Config{
			Pool:             pool,
			AppOrigin:        localOrigin,
			Now:              func() time.Time { return now },
			TrustedProxyHops: 1,
			OAuth: oauth.Resolve(oauth.Env{
				AppOrigin: localOrigin,
				Mode:      "fake",
				FakeBase:  idp.URL,
			}),
		})

		req := httptest.NewRequest(http.MethodGet, "/api/auth/oauth/google/start", nil)
		req.RemoteAddr = "10.0.0.1:1234"
		req.Header.Set("X-Forwarded-For", "9.9.9.9, 203.0.113.55")
		rec := httptest.NewRecorder()
		h.ServeHTTP(rec, req)
		if rec.Code != http.StatusFound {
			t.Fatalf("start = %d, want 302", rec.Code)
		}

		var staleLeft int
		if err := pool.QueryRow(context.Background(),
			`SELECT count(*) FROM rate_limits WHERE key = 'oauth_start:stale'`).Scan(&staleLeft); err != nil || staleLeft != 0 {
			t.Fatalf("stale rate_limits row count = %d (err=%v), want 0", staleLeft, err)
		}
		var realKeyCount int
		if err := pool.QueryRow(context.Background(),
			`SELECT count(*) FROM rate_limits WHERE key = 'oauth_start:203.0.113.55'`).Scan(&realKeyCount); err != nil || realKeyCount != 1 {
			t.Fatalf("expected oauth_start:203.0.113.55 rate row, got %d (err=%v)", realKeyCount, err)
		}
	})

	t.Run("InsertOAuthAccount returns 1 then 0 rows on conflict", func(t *testing.T) {
		ctx := context.Background()
		q := gen.New(pool)
		uid1, err := q.CreateUser(ctx, gen.CreateUserParams{Email: "race1@example.test", PasswordHash: auth.OAuthOnlyPasswordHash})
		if err != nil {
			t.Fatal(err)
		}
		uid2, err := q.CreateUser(ctx, gen.CreateUserParams{Email: "race2@example.test", PasswordHash: auth.OAuthOnlyPasswordHash})
		if err != nil {
			t.Fatal(err)
		}

		n1, err := q.InsertOAuthAccount(ctx, gen.InsertOAuthAccountParams{
			Provider: "google",
			Subject:  "sub-race-1",
			UserID:   uid1,
		})
		if err != nil || n1 != 1 {
			t.Fatalf("first InsertOAuthAccount = (%d, %v), want (1, nil)", n1, err)
		}

		n2, err := q.InsertOAuthAccount(ctx, gen.InsertOAuthAccountParams{
			Provider: "google",
			Subject:  "sub-race-1",
			UserID:   uid2,
		})
		if err != nil || n2 != 0 {
			t.Fatalf("second InsertOAuthAccount = (%d, %v), want (0, nil)", n2, err)
		}

		gotUID, err := q.GetOAuthAccountUserID(ctx, gen.GetOAuthAccountUserIDParams{
			Provider: "google",
			Subject:  "sub-race-1",
		})
		if err != nil || gotUID != uid1 {
			t.Fatalf("winning user_id = (%d, %v), want %d", gotUID, err, uid1)
		}
	})

	t.Run("callback outbound deadline redirects to error page", func(t *testing.T) {
		idp := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
			if r.URL.Path == "/google/token" {
				time.Sleep(150 * time.Millisecond)
				_, _ = w.Write([]byte(`{"access_token":"late"}`))
				return
			}
			w.WriteHeader(http.StatusNotFound)
		}))
		defer idp.Close()

		var logs syncBuffer
		h := api.NewHandler(api.Config{
			Pool:      pool,
			AppOrigin: localOrigin,
			Log:       slog.New(slog.NewJSONHandler(&logs, nil)),
			OAuth: oauth.Resolve(oauth.Env{
				AppOrigin: localOrigin,
				Mode:      "fake",
				FakeBase:  idp.URL,
			}),
		})

		startReq := httptest.NewRequest(http.MethodGet, "/api/auth/oauth/google/start", nil)
		startReq.RemoteAddr = "198.51.100.20:1234"
		startRec := httptest.NewRecorder()
		h.ServeHTTP(startRec, startReq)
		authURL, _ := url.Parse(startRec.Header().Get("Location"))
		state := authURL.Query().Get("state")
		var stateCookie *http.Cookie
		for _, c := range startRec.Result().Cookies() {
			if c.Name == "__Host-oauth" {
				stateCookie = c
			}
		}

		ctx, cancel := context.WithTimeout(context.Background(), 50*time.Millisecond)
		defer cancel()
		cbReq := httptest.NewRequest(http.MethodGet, "/api/auth/oauth/google/callback?code=c1&state="+state, nil).WithContext(ctx)
		cbReq.RemoteAddr = "198.51.100.20:1235"
		cbReq.AddCookie(stateCookie)
		cbRec := httptest.NewRecorder()
		h.ServeHTTP(cbRec, cbReq)

		if cbRec.Code != http.StatusFound || cbRec.Header().Get("Location") != "/login?error=oauth" {
			t.Fatalf("timed out callback = %d Location=%q, want 302 /login?error=oauth", cbRec.Code, cbRec.Header().Get("Location"))
		}
		for _, c := range cbRec.Result().Cookies() {
			if c.Name == "__Host-session" {
				t.Fatal("timed out callback must not set session cookie")
			}
		}
		if !strings.Contains(logs.String(), `"reason":"token_exchange_failed"`) {
			t.Fatalf("expected token_exchange_failed log, got:\n%s", logs.String())
		}
	})
}

type spyCaptcha struct {
	gotIP string
}

func (s *spyCaptcha) Verify(_ context.Context, _, remoteIP string) bool {
	s.gotIP = remoteIP
	return false
}

func TestClientIPTrustedProxyHops(t *testing.T) {
	cases := []struct {
		name       string
		hops       int
		xff        []string
		remoteAddr string
		want       string
	}{
		{
			name:       "hops=1 ignores spoofed leading entry",
			hops:       1,
			xff:        []string{"9.9.9.9, 203.0.113.7"},
			remoteAddr: "10.0.0.1:1234",
			want:       "203.0.113.7",
		},
		{
			name:       "hops=0 ignores present XFF",
			hops:       0,
			xff:        []string{"203.0.113.7"},
			remoteAddr: "198.51.100.5:4321",
			want:       "198.51.100.5",
		},
		{
			name:       "negative hops treated as 0",
			hops:       -2,
			xff:        []string{"203.0.113.7"},
			remoteAddr: "198.51.100.5:4321",
			want:       "198.51.100.5",
		},
		{
			name:       "invalid XFF entry falls back to RemoteAddr",
			hops:       1,
			xff:        []string{"9.9.9.9, not-an-ip"},
			remoteAddr: "198.51.100.5:4321",
			want:       "198.51.100.5",
		},
		{
			name:       "fewer entries than hops falls back to RemoteAddr",
			hops:       2,
			xff:        []string{"203.0.113.7"},
			remoteAddr: "198.51.100.5:4321",
			want:       "198.51.100.5",
		},
		{
			name:       "hops=2 picks second from right across multiple headers",
			hops:       2,
			xff:        []string{"9.9.9.9", "203.0.113.7, 10.0.0.2"},
			remoteAddr: "10.0.0.1:1234",
			want:       "203.0.113.7",
		},
		{
			name:       "IPv6 in XFF works",
			hops:       1,
			xff:        []string{"9.9.9.9, 2001:0db8::1"},
			remoteAddr: "[::1]:1234",
			want:       "2001:db8::1",
		},
		{
			name:       "IPv6 RemoteAddr fallback strips port",
			hops:       0,
			xff:        []string{"203.0.113.7"},
			remoteAddr: "[2001:db8::2]:4321",
			want:       "2001:db8::2",
		},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			spy := &spyCaptcha{}
			h := api.NewHandler(api.Config{
				AppOrigin:        testOrigin,
				Captcha:          spy,
				TrustedProxyHops: tc.hops,
			})
			req := httptest.NewRequest(http.MethodPost, "/api/auth/password/forgot",
				strings.NewReader(`{"email":"u@example.test","captcha":"tok"}`))
			req.Header.Set("Content-Type", "application/json")
			req.Header.Set("Origin", testOrigin)
			req.RemoteAddr = tc.remoteAddr
			for _, v := range tc.xff {
				req.Header.Add("X-Forwarded-For", v)
			}
			rec := httptest.NewRecorder()
			h.ServeHTTP(rec, req)
			if rec.Code != http.StatusForbidden {
				t.Fatalf("status = %d, want 403", rec.Code)
			}
			if spy.gotIP != tc.want {
				t.Fatalf("clientIP = %q, want %q", spy.gotIP, tc.want)
			}
		})
	}
}
