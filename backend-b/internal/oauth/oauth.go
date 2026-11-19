// Package oauth implements OAuth 2.0 Authorization Code + PKCE (S256) for Google and GitHub
// using only Go's standard library (plan pilot-oauth D3).
package oauth

import (
	"context"
	"crypto/rand"
	"crypto/sha256"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"log/slog"
	"net/http"
	netmail "net/mail"
	"net/url"
	"os"
	"strconv"
	"strings"
	"time"
)

// Provider identifies a supported OAuth 2.0 identity provider.
type Provider string

const (
	ProviderGoogle Provider = "google"
	ProviderGitHub Provider = "github"
)

// ParseProvider validates a route parameter against the supported providers.
func ParseProvider(raw string) (Provider, bool) {
	switch Provider(raw) {
	case ProviderGoogle:
		return ProviderGoogle, true
	case ProviderGitHub:
		return ProviderGitHub, true
	default:
		return "", false
	}
}

const (
	GoogleAuthorizeURL = "https://accounts.google.com/o/oauth2/v2/auth"
	GoogleTokenURL     = "https://oauth2.googleapis.com/token"
	GoogleUserInfoURL  = "https://openidconnect.googleapis.com/v1/userinfo"
	GoogleScope        = "openid email profile"

	GitHubAuthorizeURL = "https://github.com/login/oauth/authorize"
	GitHubTokenURL     = "https://github.com/login/oauth/access_token"
	GitHubUserURL      = "https://api.github.com/user"
	GitHubEmailsURL    = "https://api.github.com/user/emails"
	GitHubScope        = "read:user user:email"

	FakeClientID     = "fake-client"
	FakeClientSecret = "fake-secret"

	OutboundTimeout = 5 * time.Second
	maxResponseBody = 1 << 20 // 1 MiB cap for outbound provider responses
	userAgent       = "pilot-shortlink"
)

var (
	ErrMissingCredentials = errors.New("oauth: missing credentials")
	ErrTokenExchange      = errors.New("oauth: token exchange failed")
	ErrIdentityFetch      = errors.New("oauth: identity fetch failed")
)

// IsLocalOrigin reports whether origin is http://localhost[:port] or http://127.0.0.1[:port].
// Fake OAuth and Turnstile modes are only honoured on local origins (I6, AC-O11).
func IsLocalOrigin(origin string) bool {
	trimmed := strings.TrimRight(strings.TrimSpace(origin), "/")
	if trimmed == "" {
		return false
	}
	u, err := url.Parse(trimmed)
	if err != nil {
		return false
	}
	if u.Scheme != "http" || u.User != nil || u.Path != "" || u.RawQuery != "" || u.Fragment != "" {
		return false
	}
	host := strings.ToLower(u.Hostname())
	if host != "localhost" && host != "127.0.0.1" {
		return false
	}
	if p := u.Port(); p != "" {
		for i := 0; i < len(p); i++ {
			if p[i] < '0' || p[i] > '9' {
				return false
			}
		}
	}
	return true
}

// IsFakeEnabled reports whether OAUTH_MODE=fake is permitted for appOrigin (I6).
func IsFakeEnabled(mode, appOrigin string) bool {
	return mode == "fake" && IsLocalOrigin(appOrigin)
}

// PKCEChallenge returns the RFC 7636 S256 challenge: BASE64URL-UNPADDED(SHA256(verifier)).
func PKCEChallenge(verifier string) string {
	sum := sha256.Sum256([]byte(verifier))
	return base64.RawURLEncoding.EncodeToString(sum[:])
}

// NewStateAndVerifier generates a 32-byte random state, a 32-byte random PKCE verifier
// (both base64url without padding), and the S256 code challenge.
func NewStateAndVerifier() (state, verifier, challenge string, err error) {
	sb := make([]byte, 32)
	if _, err = rand.Read(sb); err != nil {
		return "", "", "", err
	}
	vb := make([]byte, 32)
	if _, err = rand.Read(vb); err != nil {
		return "", "", "", err
	}
	state = base64.RawURLEncoding.EncodeToString(sb)
	verifier = base64.RawURLEncoding.EncodeToString(vb)
	return state, verifier, PKCEChallenge(verifier), nil
}

// RedirectURI builds the fixed callback URL from APP_ORIGIN (never from request headers, I5).
func RedirectURI(appOrigin string, p Provider) string {
	return strings.TrimRight(strings.TrimSpace(appOrigin), "/") + "/api/auth/oauth/" + string(p) + "/callback"
}

// Identity is the normalized user identity fetched from an OAuth provider.
type Identity struct {
	Subject       string
	Email         string
	EmailVerified bool
}

// GitHubEmail represents one element of GitHub's GET /user/emails response.
type GitHubEmail struct {
	Email    string `json:"email"`
	Primary  bool   `json:"primary"`
	Verified bool   `json:"verified"`
}

// ProviderConfig holds endpoints and credentials for one OAuth provider.
type ProviderConfig struct {
	Provider     Provider
	AuthorizeURL string
	TokenURL     string
	UserInfoURL  string
	EmailsURL    string // GitHub only
	Scope        string
	ClientID     string
	ClientSecret string
	Client       *http.Client
}

// LogValue redacts ClientSecret if a ProviderConfig is ever passed to slog (I7).
func (p ProviderConfig) LogValue() slog.Value {
	return slog.GroupValue(
		slog.String("provider", string(p.Provider)),
		slog.String("authorize_url", p.AuthorizeURL),
		slog.String("client_secret", "[redacted]"),
	)
}

// String redacts ClientSecret if formatted via fmt (I7).
func (p ProviderConfig) String() string {
	return "oauth.ProviderConfig{provider=" + string(p.Provider) + ", client_secret=[redacted]}"
}

// GoString redacts ClientSecret if formatted via %#v (I7).
func (p ProviderConfig) GoString() string {
	return fmt.Sprintf(
		"oauth.ProviderConfig{Provider:%q, AuthorizeURL:%q, TokenURL:%q, UserInfoURL:%q, EmailsURL:%q, Scope:%q, ClientID:%q, ClientSecret:%q}",
		p.Provider, p.AuthorizeURL, p.TokenURL, p.UserInfoURL, p.EmailsURL, p.Scope, p.ClientID, "[redacted]",
	)
}

func (p *ProviderConfig) httpClient() *http.Client {
	if p.Client != nil {
		return p.Client
	}
	return defaultHTTPClient
}

var defaultHTTPClient = &http.Client{
	Timeout: OutboundTimeout,
	CheckRedirect: func(*http.Request, []*http.Request) error {
		return http.ErrUseLastResponse
	},
}

// Providers resolves configured providers for the HTTP handlers.
type Providers struct {
	byName map[Provider]*ProviderConfig
}

// Get returns the configuration for p, or nil when credentials are not configured.
func (ps *Providers) Get(p Provider) *ProviderConfig {
	if ps == nil {
		return nil
	}
	return ps.byName[p]
}

// Env holds raw OAuth configuration strings (typically from environment variables).
type Env struct {
	AppOrigin          string
	Mode               string
	FakeBase           string
	GoogleClientID     string
	GoogleClientSecret string
	GitHubClientID     string
	GitHubClientSecret string
}

// LogValue redacts client secrets if Env is ever passed to slog (I7).
func (e Env) LogValue() slog.Value {
	return slog.GroupValue(
		slog.String("app_origin", e.AppOrigin),
		slog.String("mode", e.Mode),
		slog.String("fake_base", e.FakeBase),
		slog.String("google_client_id", e.GoogleClientID),
		slog.String("google_client_secret", "[redacted]"),
		slog.String("github_client_id", e.GitHubClientID),
		slog.String("github_client_secret", "[redacted]"),
	)
}

// String redacts client secrets when formatted via %v / %+v (I7).
func (e Env) String() string {
	return fmt.Sprintf(
		"oauth.Env{AppOrigin:%q Mode:%q FakeBase:%q GoogleClientID:%q GoogleClientSecret:%q GitHubClientID:%q GitHubClientSecret:%q}",
		e.AppOrigin, e.Mode, e.FakeBase, e.GoogleClientID, "[redacted]", e.GitHubClientID, "[redacted]",
	)
}

// GoString redacts client secrets when formatted via %#v (I7).
func (e Env) GoString() string {
	return fmt.Sprintf(
		"oauth.Env{AppOrigin:%q, Mode:%q, FakeBase:%q, GoogleClientID:%q, GoogleClientSecret:%q, GitHubClientID:%q, GitHubClientSecret:%q}",
		e.AppOrigin, e.Mode, e.FakeBase, e.GoogleClientID, "[redacted]", e.GitHubClientID, "[redacted]",
	)
}

// FromEnv builds Providers from process environment variables and appOrigin.
func FromEnv(appOrigin string) *Providers {
	return Resolve(Env{
		AppOrigin:          appOrigin,
		Mode:               os.Getenv("OAUTH_MODE"),
		FakeBase:           os.Getenv("OAUTH_FAKE_BASE"),
		GoogleClientID:     os.Getenv("GOOGLE_CLIENT_ID"),
		GoogleClientSecret: os.Getenv("GOOGLE_CLIENT_SECRET"),
		GitHubClientID:     os.Getenv("GITHUB_CLIENT_ID"),
		GitHubClientSecret: os.Getenv("GITHUB_CLIENT_SECRET"),
	})
}

// Resolve builds Providers from explicit configuration values.
// Fake mode is honoured only when APP_ORIGIN is localhost or 127.0.0.1 (I6, AC-O11).
func Resolve(e Env) *Providers {
	out := &Providers{byName: map[Provider]*ProviderConfig{}}
	if IsFakeEnabled(e.Mode, e.AppOrigin) {
		base := strings.TrimRight(strings.TrimSpace(e.FakeBase), "/")
		if base == "" {
			return out
		}
		out.byName[ProviderGoogle] = &ProviderConfig{
			Provider:     ProviderGoogle,
			AuthorizeURL: base + "/google/authorize",
			TokenURL:     base + "/google/token",
			UserInfoURL:  base + "/google/userinfo",
			Scope:        GoogleScope,
			ClientID:     FakeClientID,
			ClientSecret: FakeClientSecret,
		}
		out.byName[ProviderGitHub] = &ProviderConfig{
			Provider:     ProviderGitHub,
			AuthorizeURL: base + "/github/login/oauth/authorize",
			TokenURL:     base + "/github/login/oauth/access_token",
			UserInfoURL:  base + "/github/user",
			EmailsURL:    base + "/github/user/emails",
			Scope:        GitHubScope,
			ClientID:     FakeClientID,
			ClientSecret: FakeClientSecret,
		}
		return out
	}

	if id, sec := strings.TrimSpace(e.GoogleClientID), strings.TrimSpace(e.GoogleClientSecret); id != "" && sec != "" {
		out.byName[ProviderGoogle] = &ProviderConfig{
			Provider:     ProviderGoogle,
			AuthorizeURL: GoogleAuthorizeURL,
			TokenURL:     GoogleTokenURL,
			UserInfoURL:  GoogleUserInfoURL,
			Scope:        GoogleScope,
			ClientID:     id,
			ClientSecret: sec,
		}
	}
	if id, sec := strings.TrimSpace(e.GitHubClientID), strings.TrimSpace(e.GitHubClientSecret); id != "" && sec != "" {
		out.byName[ProviderGitHub] = &ProviderConfig{
			Provider:     ProviderGitHub,
			AuthorizeURL: GitHubAuthorizeURL,
			TokenURL:     GitHubTokenURL,
			UserInfoURL:  GitHubUserURL,
			EmailsURL:    GitHubEmailsURL,
			Scope:        GitHubScope,
			ClientID:     id,
			ClientSecret: sec,
		}
	}
	return out
}

// BuildAuthorizeURL returns the provider authorization URL with PKCE S256 parameters (AC-O6).
func (p *ProviderConfig) BuildAuthorizeURL(redirectURI, state, challenge string) (string, error) {
	u, err := url.Parse(p.AuthorizeURL)
	if err != nil || u.Scheme == "" || u.Host == "" {
		return "", ErrMissingCredentials
	}
	q := u.Query()
	q.Set("response_type", "code")
	q.Set("client_id", p.ClientID)
	q.Set("redirect_uri", redirectURI)
	q.Set("scope", p.Scope)
	q.Set("state", state)
	q.Set("code_challenge", challenge)
	q.Set("code_challenge_method", "S256")
	u.RawQuery = q.Encode()
	return u.String(), nil
}

// Exchange trades an authorization code and PKCE verifier for an access token.
// Errors never include the code, verifier, secret, token, or response body (I7).
func (p *ProviderConfig) Exchange(ctx context.Context, code, redirectURI, verifier string) (string, error) {
	form := url.Values{}
	form.Set("grant_type", "authorization_code")
	form.Set("code", code)
	form.Set("redirect_uri", redirectURI)
	form.Set("client_id", p.ClientID)
	form.Set("client_secret", p.ClientSecret)
	form.Set("code_verifier", verifier)

	req, err := http.NewRequestWithContext(ctx, http.MethodPost, p.TokenURL, strings.NewReader(form.Encode()))
	if err != nil {
		return "", ErrTokenExchange
	}
	req.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	req.Header.Set("Accept", "application/json")
	req.Header.Set("User-Agent", userAgent)

	resp, err := p.httpClient().Do(req)
	if err != nil {
		return "", ErrTokenExchange
	}
	defer resp.Body.Close()

	body, err := io.ReadAll(io.LimitReader(resp.Body, maxResponseBody))
	if err != nil || resp.StatusCode != http.StatusOK {
		return "", ErrTokenExchange
	}

	var parsed struct {
		AccessToken string `json:"access_token"`
	}
	if err := json.Unmarshal(body, &parsed); err != nil || strings.TrimSpace(parsed.AccessToken) == "" {
		return "", ErrTokenExchange
	}
	return parsed.AccessToken, nil
}

// Identity fetches the normalized user identity from the provider using accessToken.
// Errors never include tokens, emails, or response bodies (I7).
func (p *ProviderConfig) Identity(ctx context.Context, accessToken string) (Identity, error) {
	switch p.Provider {
	case ProviderGoogle:
		return p.googleIdentity(ctx, accessToken)
	case ProviderGitHub:
		return p.githubIdentity(ctx, accessToken)
	default:
		return Identity{}, ErrIdentityFetch
	}
}

func (p *ProviderConfig) getJSON(ctx context.Context, endpoint, accessToken, accept string) ([]byte, error) {
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, endpoint, nil)
	if err != nil {
		return nil, ErrIdentityFetch
	}
	req.Header.Set("Authorization", "Bearer "+accessToken)
	req.Header.Set("Accept", accept)
	req.Header.Set("User-Agent", userAgent)

	resp, err := p.httpClient().Do(req)
	if err != nil {
		return nil, ErrIdentityFetch
	}
	defer resp.Body.Close()

	body, err := io.ReadAll(io.LimitReader(resp.Body, maxResponseBody))
	if err != nil || resp.StatusCode != http.StatusOK {
		return nil, ErrIdentityFetch
	}
	return body, nil
}

func (p *ProviderConfig) googleIdentity(ctx context.Context, accessToken string) (Identity, error) {
	body, err := p.getJSON(ctx, p.UserInfoURL, accessToken, "application/json")
	if err != nil {
		return Identity{}, ErrIdentityFetch
	}
	return ParseGoogleUserInfo(body)
}

func (p *ProviderConfig) githubIdentity(ctx context.Context, accessToken string) (Identity, error) {
	userBody, err := p.getJSON(ctx, p.UserInfoURL, accessToken, "application/vnd.github+json")
	if err != nil {
		return Identity{}, ErrIdentityFetch
	}
	var u struct {
		ID *int64 `json:"id"`
	}
	if err := json.Unmarshal(userBody, &u); err != nil || u.ID == nil || *u.ID <= 0 {
		return Identity{}, ErrIdentityFetch
	}

	emailsBody, err := p.getJSON(ctx, p.EmailsURL, accessToken, "application/vnd.github+json")
	if err != nil {
		return Identity{}, ErrIdentityFetch
	}
	var emails []GitHubEmail
	if err := json.Unmarshal(emailsBody, &emails); err != nil {
		return Identity{}, ErrIdentityFetch
	}

	email, verified := SelectGitHubEmail(emails)
	return Identity{
		Subject:       strconv.FormatInt(*u.ID, 10),
		Email:         email,
		EmailVerified: verified,
	}, nil
}

// ParseGoogleUserInfo decodes Google's OIDC userinfo JSON into a normalized Identity.
func ParseGoogleUserInfo(body []byte) (Identity, error) {
	var info struct {
		Sub           string `json:"sub"`
		Email         string `json:"email"`
		EmailVerified bool   `json:"email_verified"`
	}
	if err := json.Unmarshal(body, &info); err != nil {
		return Identity{}, ErrIdentityFetch
	}
	sub := strings.TrimSpace(info.Sub)
	if sub == "" {
		return Identity{}, ErrIdentityFetch
	}
	email, ok := normalizeEmail(info.Email)
	return Identity{
		Subject:       sub,
		Email:         email,
		EmailVerified: info.EmailVerified && ok,
	}, nil
}

// SelectGitHubEmail returns the lowercase primary verified email from GitHub's /user/emails list (AC-O2, I1).
func SelectGitHubEmail(emails []GitHubEmail) (string, bool) {
	for _, e := range emails {
		if e.Primary && e.Verified {
			if addr, ok := normalizeEmail(e.Email); ok {
				return addr, true
			}
		}
	}
	return "", false
}

func normalizeEmail(raw string) (string, bool) {
	email := strings.ToLower(strings.TrimSpace(raw))
	if addr, err := netmail.ParseAddress(email); err != nil || addr.Address != email || len(email) > 254 {
		return "", false
	}
	return email, true
}
