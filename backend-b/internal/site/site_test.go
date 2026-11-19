package site

import (
	"strings"
	"testing"
)

func TestCSPAllowsOnlyTurnstileAdditions(t *testing.T) {
	if !strings.Contains(CSP, "script-src 'self' https://challenges.cloudflare.com;") {
		t.Fatalf("CSP script-src mismatch: %s", CSP)
	}
	if !strings.Contains(CSP, "frame-src https://challenges.cloudflare.com;") {
		t.Fatalf("CSP frame-src missing: %s", CSP)
	}
	if strings.Contains(CSP, "unsafe-inline") || strings.Contains(CSP, "unsafe-eval") {
		t.Fatalf("CSP must not enable unsafe-inline or unsafe-eval: %s", CSP)
	}
}
