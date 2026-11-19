package captcha

import (
	"context"
	"fmt"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync/atomic"
	"testing"
	"time"
)

func TestFakeVerifier(t *testing.T) {
	v := Fake{}
	if !v.Verify(context.Background(), FakePassToken, "1.2.3.4") {
		t.Fatal("Fake rejected fake-pass")
	}
	for _, bad := range []string{"", "nope", "fake-pass ", "FAKE-PASS"} {
		if v.Verify(context.Background(), bad, "") {
			t.Fatalf("Fake accepted %q", bad)
		}
	}
}

func TestResolveFakeModeGuardAndSiteKey(t *testing.T) {
	// Local origin + TURNSTILE_MODE=fake -> Fake adapter and nil site key even when site key is configured.
	v, key := Resolve("http://localhost:8080", "fake", "sec", "0x4AAA")
	if _, ok := v.(Fake); !ok || key != nil {
		t.Fatalf("local fake: v=%T key=%v, want Fake + nil", v, key)
	}

	// Non-local origin + TURNSTILE_MODE=fake -> fake mode ignored, Cloudflare adapter and configured site key returned.
	v, key = Resolve("https://pilot-shortlink-b.onrender.com", "fake", "sec", "0x4AAA")
	if _, ok := v.(*Cloudflare); !ok || key == nil || *key != "0x4AAA" {
		t.Fatalf("non-local fake: v=%T key=%v, want *Cloudflare + 0x4AAA", v, key)
	}

	// Non-local origin + empty site key -> nil site key.
	v, key = Resolve("https://pilot-shortlink-b.onrender.com", "", "sec", "  ")
	if _, ok := v.(*Cloudflare); !ok || key != nil {
		t.Fatalf("empty site key: v=%T key=%v, want *Cloudflare + nil", v, key)
	}
}

func TestCloudflareVerifier(t *testing.T) {
	t.Run("missing secret fails closed without HTTP call", func(t *testing.T) {
		var hits atomic.Int32
		srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
			hits.Add(1)
			_, _ = w.Write([]byte(`{"success":true}`))
		}))
		defer srv.Close()

		c := NewCloudflare("")
		c.URL = srv.URL
		if c.Verify(context.Background(), "tok", "1.2.3.4") || hits.Load() != 0 {
			t.Fatalf("missing secret should fail closed without hitting HTTP, hits=%d", hits.Load())
		}
	})

	t.Run("sends form fields and requires success true", func(t *testing.T) {
		var gotSecret, gotResp, gotIP string
		var reply string
		srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
			_ = r.ParseForm()
			gotSecret = r.Form.Get("secret")
			gotResp = r.Form.Get("response")
			gotIP = r.Form.Get("remoteip")
			_, _ = w.Write([]byte(reply))
		}))
		defer srv.Close()

		c := NewCloudflare("super-secret")
		c.URL = srv.URL
		if strings.Contains(fmt.Sprint(c), "super-secret") {
			t.Fatalf("Cloudflare.String leaked secret: %s", fmt.Sprint(c))
		}

		reply = `{"success":true}`
		if !c.Verify(context.Background(), "tok-1", "203.0.113.5") {
			t.Fatal("expected success=true to pass")
		}
		if gotSecret != "super-secret" || gotResp != "tok-1" || gotIP != "203.0.113.5" {
			t.Fatalf("form fields = (%q, %q, %q)", gotSecret, gotResp, gotIP)
		}

		for _, bad := range []string{`{"success":false}`, `{}`, `not-json`} {
			reply = bad
			if c.Verify(context.Background(), "tok-1", "203.0.113.5") {
				t.Fatalf("body %q should fail closed", bad)
			}
		}
	})

	t.Run("timeout and non-200 fail closed", func(t *testing.T) {
		srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
			if r.URL.Path == "/slow" {
				time.Sleep(150 * time.Millisecond)
			}
			w.WriteHeader(http.StatusInternalServerError)
		}))
		defer srv.Close()

		c := NewCloudflare("sec")
		c.Client = &http.Client{Timeout: 50 * time.Millisecond}
		c.URL = srv.URL + "/slow"
		if c.Verify(context.Background(), "tok", "") {
			t.Fatal("timeout should fail closed")
		}
		c.URL = srv.URL + "/err"
		if c.Verify(context.Background(), "tok", "") {
			t.Fatal("500 should fail closed")
		}
	})
}
