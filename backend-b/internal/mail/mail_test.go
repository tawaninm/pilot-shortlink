package mail

import (
	"context"
	"encoding/json"
	"errors"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
)

func TestBuild(t *testing.T) {
	token := strings.Repeat("a", 43)
	hash := "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"

	v := Build(PurposeVerify, "u@example.test", "https://app.example/", token, hash)
	if v.Subject != "ยืนยันอีเมลสำหรับ pilot-shortlink" {
		t.Fatalf("verify subject = %q", v.Subject)
	}
	if !strings.Contains(v.Text, "https://app.example/verify-email?token="+token) || !strings.Contains(v.Text, "30") {
		t.Fatalf("verify text = %q", v.Text)
	}
	if v.IdempotencyKey != "verify/0123456789abcdef" {
		t.Fatalf("verify idempotency key = %q", v.IdempotencyKey)
	}

	r := Build(PurposeReset, "u@example.test", "https://app.example", token, hash)
	if r.Subject != "ตั้งรหัสผ่านใหม่สำหรับ pilot-shortlink" {
		t.Fatalf("reset subject = %q", r.Subject)
	}
	if !strings.Contains(r.Text, "https://app.example/reset-password?token="+token) || !strings.Contains(r.HTML, "https://app.example/reset-password?token="+token) {
		t.Fatalf("reset body = %q / %q", r.Text, r.HTML)
	}
	if r.IdempotencyKey != "reset/0123456789abcdef" {
		t.Fatalf("reset idempotency key = %q", r.IdempotencyKey)
	}
}

func TestCapture(t *testing.T) {
	c := NewCapture()
	if got := c.For("nobody@example.test"); len(got) != 0 {
		t.Fatalf("empty capture returned %v", got)
	}

	_ = c.Send(context.Background(), Mail{To: "a@example.test", Subject: "first", Text: "1", HTML: "<p>1</p>"})
	_ = c.Send(context.Background(), Mail{To: "b@example.test", Subject: "other", Text: "x", HTML: "<p>x</p>"})
	_ = c.Send(context.Background(), Mail{To: "A@example.test", Subject: "second", Text: "2", HTML: "<p>2</p>"})

	got := c.For("a@example.test")
	if len(got) != 2 || got[0].Subject != "first" || got[1].Subject != "second" {
		t.Fatalf("For(a) = %+v, want oldest first [first, second]", got)
	}

	var wg sync.WaitGroup
	for range 10 {
		wg.Add(1)
		go func() {
			defer wg.Done()
			_ = c.Send(context.Background(), Mail{To: "c@example.test", Subject: "s"})
			_ = c.For("c@example.test")
		}()
	}
	wg.Wait()
	if len(c.For("c@example.test")) != 10 {
		t.Fatalf("concurrent capture count = %d", len(c.For("c@example.test")))
	}
}

func TestFromEnv(t *testing.T) {
	t.Setenv("MAIL_MODE", "")
	t.Setenv("RESEND_API_KEY", "re_test")
	t.Setenv("MAIL_FROM", "")
	m, capStore := FromEnv()
	if capStore != nil {
		t.Fatal("AC-V8: capture store must be nil when MAIL_MODE is not capture")
	}
	if _, ok := m.(*Resend); !ok {
		t.Fatalf("default mailer = %T, want *Resend", m)
	}

	t.Setenv("MAIL_MODE", "capture")
	m, capStore = FromEnv()
	if capStore == nil || m != capStore {
		t.Fatalf("MAIL_MODE=capture: m=%v capStore=%v", m, capStore)
	}
}

func TestResend(t *testing.T) {
	t.Run("missing API key skips HTTP call", func(t *testing.T) {
		var hits atomic.Int32
		srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
			hits.Add(1)
			w.WriteHeader(http.StatusOK)
		}))
		defer srv.Close()

		r := NewResend("", "")
		r.URL = srv.URL
		err := r.Send(context.Background(), Mail{To: "a@example.test", Subject: "s", Text: "t", HTML: "h"})
		if !errors.Is(err, ErrMissingAPIKey) || hits.Load() != 0 {
			t.Fatalf("err=%v hits=%d", err, hits.Load())
		}
	})

	t.Run("sends expected headers and payload", func(t *testing.T) {
		var gotAuth, gotIdem string
		var gotBody resendRequest
		srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, req *http.Request) {
			gotAuth = req.Header.Get("Authorization")
			gotIdem = req.Header.Get("Idempotency-Key")
			_ = json.NewDecoder(req.Body).Decode(&gotBody)
			w.WriteHeader(http.StatusOK)
		}))
		defer srv.Close()

		r := NewResend("re_secret", "")
		r.URL = srv.URL
		err := r.Send(context.Background(), Mail{
			To: "a@example.test", Subject: "sub", Text: "txt", HTML: "<p>txt</p>", IdempotencyKey: "verify/0123456789abcdef",
		})
		if err != nil {
			t.Fatal(err)
		}
		if gotAuth != "Bearer re_secret" || gotIdem != "verify/0123456789abcdef" {
			t.Fatalf("headers: auth=%q idem=%q", gotAuth, gotIdem)
		}
		if gotBody.From != DefaultFrom || len(gotBody.To) != 1 || gotBody.To[0] != "a@example.test" {
			t.Fatalf("payload: %+v", gotBody)
		}
	})

	t.Run("non-2xx returns error without leaking body", func(t *testing.T) {
		for _, code := range []int{http.StatusInternalServerError, http.StatusTooManyRequests} {
			srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
				w.WriteHeader(code)
				_, _ = w.Write([]byte(`{"message":"failed for secret@example.test"}`))
			}))
			r := NewResend("re_secret", "custom <no-reply@example.test>")
			r.URL = srv.URL
			err := r.Send(context.Background(), Mail{To: "secret@example.test", Subject: "s"})
			srv.Close()
			if err == nil || strings.Contains(err.Error(), "secret@example.test") {
				t.Fatalf("code %d: err=%v", code, err)
			}
		}
	})
}
