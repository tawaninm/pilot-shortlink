package main

import (
	"io"
	"log/slog"
	"strings"
	"testing"
)

func TestMainRefusesFakeModesOnRender(t *testing.T) {
	log := slog.New(slog.NewJSONHandler(io.Discard, nil))

	cases := []struct {
		name    string
		envKey  string
		envVal  string
		wantSub string
	}{
		{name: "MAIL_MODE=capture", envKey: "MAIL_MODE", envVal: "capture", wantSub: "MAIL_MODE=capture"},
		{name: "OAUTH_MODE=fake", envKey: "OAUTH_MODE", envVal: "fake", wantSub: "OAUTH_MODE=fake"},
		{name: "TURNSTILE_MODE=fake", envKey: "TURNSTILE_MODE", envVal: "fake", wantSub: "TURNSTILE_MODE=fake"},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			t.Setenv("DATABASE_URL", "postgres://postgres:x@127.0.0.1:1/postgres")
			t.Setenv("APP_ORIGIN", "https://pilot-shortlink-b.onrender.com")
			t.Setenv("RENDER", "true")
			t.Setenv("MAIL_MODE", "")
			t.Setenv("OAUTH_MODE", "")
			t.Setenv("TURNSTILE_MODE", "")
			t.Setenv(tc.envKey, tc.envVal)

			err := run(log)
			if err == nil || !strings.Contains(err.Error(), tc.wantSub) {
				t.Fatalf("run() with RENDER=true and %s=%s: err=%v, want error mentioning %q", tc.envKey, tc.envVal, err, tc.wantSub)
			}
		})
	}
}

func TestCheckRenderGuardsAllowsFakeWithoutRender(t *testing.T) {
	t.Setenv("RENDER", "")
	t.Setenv("MAIL_MODE", "capture")
	t.Setenv("OAUTH_MODE", "fake")
	t.Setenv("TURNSTILE_MODE", "fake")
	if err := checkRenderGuards(); err != nil {
		t.Fatalf("checkRenderGuards without RENDER returned %v, want nil", err)
	}
}
