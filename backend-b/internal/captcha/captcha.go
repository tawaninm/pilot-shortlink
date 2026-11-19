// Package captcha verifies Cloudflare Turnstile tokens with a fake adapter for local tests (plan pilot-oauth).
package captcha

import (
	"context"
	"encoding/json"
	"fmt"
	"io"
	"log/slog"
	"net/http"
	"net/url"
	"os"
	"strings"
	"time"

	"github.com/tawaninm/pilot-shortlink/backend-b/internal/oauth"
)

const (
	DefaultSiteverifyURL = "https://challenges.cloudflare.com/turnstile/v0/siteverify"
	FakePassToken        = "fake-pass"
	MaxTokenLen          = 2048
	verifyTimeout        = 5 * time.Second
	maxResponseBody      = 64 << 10
)

// Verifier validates a Turnstile response token.
type Verifier interface {
	Verify(ctx context.Context, token, remoteIP string) bool
}

// IsFakeEnabled reports whether TURNSTILE_MODE=fake is permitted for appOrigin (I6).
func IsFakeEnabled(mode, appOrigin string) bool {
	return mode == "fake" && oauth.IsLocalOrigin(appOrigin)
}

// Fake accepts only FakePassToken ("fake-pass"). Used only under the local-origin guard.
type Fake struct{}

// Verify returns true only for the exact "fake-pass" token.
func (Fake) Verify(_ context.Context, token, _ string) bool {
	return token == FakePassToken
}

// Cloudflare verifies tokens against Cloudflare Turnstile's siteverify endpoint.
// Fails closed on missing secret, empty/oversized token, network errors, timeouts, or bad JSON.
type Cloudflare struct {
	Secret string
	URL    string
	Client *http.Client
}

// NewCloudflare returns a Cloudflare verifier with a 5 s timeout.
func NewCloudflare(secret string) *Cloudflare {
	return &Cloudflare{
		Secret: strings.TrimSpace(secret),
		URL:    DefaultSiteverifyURL,
		Client: &http.Client{
			Timeout: verifyTimeout,
			CheckRedirect: func(*http.Request, []*http.Request) error {
				return http.ErrUseLastResponse
			},
		},
	}
}

// LogValue redacts Secret if a Cloudflare verifier is ever passed to slog (I7).
func (c Cloudflare) LogValue() slog.Value {
	return slog.GroupValue(slog.String("secret", "[redacted]"))
}

// String redacts Secret if formatted via fmt (I7).
func (c Cloudflare) String() string {
	return "captcha.Cloudflare{secret=[redacted]}"
}

// GoString redacts Secret if formatted via %#v (I7).
func (c Cloudflare) GoString() string {
	return fmt.Sprintf("captcha.Cloudflare{Secret:%q, URL:%q}", "[redacted]", c.URL)
}

// Verify posts secret, response, and optional remoteip to Turnstile siteverify.
func (c *Cloudflare) Verify(ctx context.Context, token, remoteIP string) bool {
	if c == nil || c.Secret == "" || token == "" || len(token) > MaxTokenLen {
		return false
	}
	endpoint := c.URL
	if endpoint == "" {
		endpoint = DefaultSiteverifyURL
	}
	client := c.Client
	if client == nil {
		client = &http.Client{
			Timeout: verifyTimeout,
			CheckRedirect: func(*http.Request, []*http.Request) error {
				return http.ErrUseLastResponse
			},
		}
	}

	form := url.Values{}
	form.Set("secret", c.Secret)
	form.Set("response", token)
	if ip := strings.TrimSpace(remoteIP); ip != "" {
		form.Set("remoteip", ip)
	}

	req, err := http.NewRequestWithContext(ctx, http.MethodPost, endpoint, strings.NewReader(form.Encode()))
	if err != nil {
		return false
	}
	req.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	req.Header.Set("Accept", "application/json")

	resp, err := client.Do(req)
	if err != nil {
		return false
	}
	defer resp.Body.Close()

	body, err := io.ReadAll(io.LimitReader(resp.Body, maxResponseBody))
	if err != nil || resp.StatusCode != http.StatusOK {
		return false
	}
	var parsed struct {
		Success bool `json:"success"`
	}
	if err := json.Unmarshal(body, &parsed); err != nil {
		return false
	}
	return parsed.Success
}

// Resolve selects the fake or Cloudflare adapter and resolves the public site key for GET /api/config.
// In fake mode (only allowed on localhost/127.0.0.1), siteKey is always nil.
func Resolve(appOrigin, mode, secret, siteKey string) (Verifier, *string) {
	if IsFakeEnabled(mode, appOrigin) {
		return Fake{}, nil
	}
	var key *string
	if trimmed := strings.TrimSpace(siteKey); trimmed != "" {
		key = &trimmed
	}
	return NewCloudflare(secret), key
}

// FromEnv configures Turnstile from TURNSTILE_MODE, TURNSTILE_SECRET, and TURNSTILE_SITE_KEY.
func FromEnv(appOrigin string) (Verifier, *string) {
	return Resolve(
		appOrigin,
		os.Getenv("TURNSTILE_MODE"),
		os.Getenv("TURNSTILE_SECRET"),
		os.Getenv("TURNSTILE_SITE_KEY"),
	)
}
