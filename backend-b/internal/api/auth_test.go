package api_test

import (
	"bytes"
	"context"
	"encoding/json"
	"log/slog"
	"net/http"
	"net/http/httptest"
	"regexp"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/tawaninm/pilot-shortlink/backend-b/internal/api"
	"github.com/tawaninm/pilot-shortlink/backend-b/internal/auth"
	"github.com/tawaninm/pilot-shortlink/backend-b/internal/mail"
	"github.com/tawaninm/pilot-shortlink/backend-b/internal/testdb"
)

const testOrigin = "https://short.example"

type syncBuffer struct {
	mu  sync.Mutex
	buf bytes.Buffer
}

func (b *syncBuffer) Write(p []byte) (int, error) {
	b.mu.Lock()
	defer b.mu.Unlock()
	return b.buf.Write(p)
}

func (b *syncBuffer) String() string {
	b.mu.Lock()
	defer b.mu.Unlock()
	return b.buf.String()
}

var mailTokenRe = regexp.MustCompile(`/(?:verify-email|reset-password)\?token=([A-Za-z0-9_-]{43})`)

func waitCapturedToken(t *testing.T, c *mail.Capture, to string, count int) string {
	t.Helper()
	deadline := time.Now().Add(3 * time.Second)
	for time.Now().Before(deadline) {
		items := c.For(to)
		if len(items) >= count {
			m := mailTokenRe.FindStringSubmatch(items[len(items)-1].Text)
			if len(m) == 2 {
				return m[1]
			}
			t.Fatalf("no token in captured mail text: %q", items[len(items)-1].Text)
		}
		time.Sleep(20 * time.Millisecond)
	}
	t.Fatalf("timed out waiting for %d mail(s) to %s", count, to)
	return ""
}

func postJSON(t *testing.T, h http.Handler, path, body string) *httptest.ResponseRecorder {
	t.Helper()
	req := httptest.NewRequest(http.MethodPost, path, strings.NewReader(body))
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("Origin", testOrigin)
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	return rec
}

func TestCaptureRouteAbsentWithoutMailMode(t *testing.T) {
	t.Setenv("MAIL_MODE", "")
	mailer, capStore := mail.FromEnv()
	h := api.NewHandler(api.Config{
		AppOrigin: testOrigin,
		Log:       slog.New(slog.NewJSONHandler(&syncBuffer{}, nil)),
		Mailer:    mailer,
		Capture:   capStore,
	})

	req := httptest.NewRequest(http.MethodGet, "/api/test/mail?to=a@example.test", nil)
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	if rec.Code != http.StatusNotFound {
		t.Fatalf("AC-V8: GET /api/test/mail without MAIL_MODE=capture returned %d, want 404", rec.Code)
	}

	c := mail.NewCapture()
	hCap := api.NewHandler(api.Config{
		AppOrigin: testOrigin,
		Log:       slog.New(slog.NewJSONHandler(&syncBuffer{}, nil)),
		Mailer:    c,
		Capture:   c,
	})
	recCap := httptest.NewRecorder()
	hCap.ServeHTTP(recCap, httptest.NewRequest(http.MethodGet, "/api/test/mail?to=a@example.test", nil))
	if recCap.Code != http.StatusOK {
		t.Fatalf("GET /api/test/mail with capture returned %d, want 200", recCap.Code)
	}
}

func TestEmailTokensAndLogging(t *testing.T) {
	pool := testdb.New(t)

	t.Run("expired verify and reset tokens return 400 token_invalid", func(t *testing.T) {
		c := mail.NewCapture()
		// Injected clock 1 hour in the past: 30-min token expiry lands 30 min in the past.
		h := api.NewHandler(api.Config{
			Pool:      pool,
			AppOrigin: testOrigin,
			Log:       slog.New(slog.NewJSONHandler(&syncBuffer{}, nil)),
			Now:       func() time.Time { return time.Now().Add(-time.Hour) },
			Mailer:    c,
			Capture:   c,
		})

		rec := postJSON(t, h, "/api/auth/register", `{"email":"exp@example.test","password":"correct-horse-battery"}`)
		if rec.Code != http.StatusCreated {
			t.Fatalf("register: %d %s", rec.Code, rec.Body.String())
		}
		verifyTok := waitCapturedToken(t, c, "exp@example.test", 1)

		vRec := postJSON(t, h, "/api/auth/verify-email", `{"token":"`+verifyTok+`"}`)
		if vRec.Code != http.StatusBadRequest || !strings.Contains(vRec.Body.String(), `"token_invalid"`) {
			t.Fatalf("expired verify token: %d %s", vRec.Code, vRec.Body.String())
		}

		fRec := postJSON(t, h, "/api/auth/password/forgot", `{"email":"exp@example.test"}`)
		if fRec.Code != http.StatusNoContent {
			t.Fatalf("forgot: %d", fRec.Code)
		}
		resetTok := waitCapturedToken(t, c, "exp@example.test", 2)

		rRec := postJSON(t, h, "/api/auth/password/reset", `{"token":"`+resetTok+`","password":"new-correct-horse-battery"}`)
		if rRec.Code != http.StatusBadRequest || !strings.Contains(rRec.Body.String(), `"token_invalid"`) {
			t.Fatalf("expired reset token: %d %s", rRec.Code, rRec.Body.String())
		}
	})

	t.Run("failing mailer (500 and 429) does not fail register and logs contain no token or PII", func(t *testing.T) {
		for _, status := range []int{http.StatusInternalServerError, http.StatusTooManyRequests} {
			var logs syncBuffer
			done := make(chan struct{}, 1)
			resendSrv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
				w.WriteHeader(status)
				_, _ = w.Write([]byte(`{"message":"provider error"}`))
				select {
				case done <- struct{}{}:
				default:
				}
			}))

			rm := mail.NewResend("re_test_key", "")
			rm.URL = resendSrv.URL
			h := api.NewHandler(api.Config{
				Pool:      pool,
				AppOrigin: testOrigin,
				Log:       slog.New(slog.NewJSONHandler(&logs, nil)),
				Mailer:    rm,
			})

			email := "fail" + time.Now().Format("150405.000") + "@example.test"
			if status == http.StatusTooManyRequests {
				email = "fail429@example.test"
			}
			rec := postJSON(t, h, "/api/auth/register", `{"email":"`+email+`","password":"correct-horse-battery"}`)
			if rec.Code != http.StatusCreated {
				resendSrv.Close()
				t.Fatalf("register with Resend %d returned %d: %s", status, rec.Code, rec.Body.String())
			}
			var body struct {
				Email         string `json:"email"`
				EmailVerified bool   `json:"emailVerified"`
			}
			if err := json.Unmarshal(rec.Body.Bytes(), &body); err != nil || body.Email != email || body.EmailVerified {
				resendSrv.Close()
				t.Fatalf("register body = %+v, err=%v", body, err)
			}

			select {
			case <-done:
			case <-time.After(3 * time.Second):
				resendSrv.Close()
				t.Fatal("timed out waiting for background mail send")
			}
			// Give the goroutine a brief moment to write the log line after the HTTP response.
			deadline := time.Now().Add(time.Second)
			for time.Now().Before(deadline) && !strings.Contains(logs.String(), `"mail_failed"`) {
				time.Sleep(10 * time.Millisecond)
			}
			resendSrv.Close()

			out := logs.String()
			if !strings.Contains(out, `"mail_failed"`) || !strings.Contains(out, `"purpose":"verify"`) {
				t.Fatalf("expected mail_failed log with purpose=verify, got:\n%s", out)
			}
			for _, forbidden := range []string{email, "correct-horse-battery", "/verify-email?token=", "re_test_key"} {
				if strings.Contains(out, forbidden) {
					t.Fatalf("log leaked %q:\n%s", forbidden, out)
				}
			}
		}
	})

	t.Run("successful send logs purpose only and never logs the token", func(t *testing.T) {
		var logs syncBuffer
		c := mail.NewCapture()
		h := api.NewHandler(api.Config{
			Pool:      pool,
			AppOrigin: testOrigin,
			Log:       slog.New(slog.NewJSONHandler(&logs, nil)),
			Mailer:    c,
			Capture:   c,
		})

		email := "nolog@example.test"
		rec := postJSON(t, h, "/api/auth/register", `{"email":"`+email+`","password":"correct-horse-battery"}`)
		if rec.Code != http.StatusCreated {
			t.Fatalf("register: %d", rec.Code)
		}
		tok := waitCapturedToken(t, c, email, 1)
		hash := auth.HashToken(tok)

		deadline := time.Now().Add(time.Second)
		for time.Now().Before(deadline) && !strings.Contains(logs.String(), `"mail_sent"`) {
			time.Sleep(10 * time.Millisecond)
		}

		out := logs.String()
		if !strings.Contains(out, `"mail_sent"`) || !strings.Contains(out, `"purpose":"verify"`) {
			t.Fatalf("expected mail_sent log with purpose=verify, got:\n%s", out)
		}
		for _, forbidden := range []string{tok, hash, hash[:16], email, "correct-horse-battery", "/verify-email?token="} {
			if strings.Contains(out, forbidden) {
				t.Fatalf("log leaked %q:\n%s", forbidden, out)
			}
		}

		// Also verify a row whose expires_at is directly in the past is rejected.
		if _, err := pool.Exec(context.Background(), `UPDATE email_tokens SET expires_at = now() - interval '1 minute' WHERE token_hash = $1`, hash); err != nil {
			t.Fatal(err)
		}
		vRec := postJSON(t, h, "/api/auth/verify-email", `{"token":"`+tok+`"}`)
		if vRec.Code != http.StatusBadRequest {
			t.Fatalf("past expires_at verify: %d", vRec.Code)
		}
	})
}
