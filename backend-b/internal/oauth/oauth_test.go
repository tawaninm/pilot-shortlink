package oauth

import (
	"context"
	"fmt"
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"testing"
)

func TestPKCEChallengeRFC7636Vector(t *testing.T) {
	// RFC 7636 Appendix B test vector.
	const (
		verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"
		want     = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
	)
	if got := PKCEChallenge(verifier); got != want {
		t.Fatalf("PKCEChallenge(%q) = %q, want %q", verifier, got, want)
	}
}

func TestIsLocalOriginAndFakeGuard(t *testing.T) {
	local := []string{
		"http://localhost",
		"http://localhost:8080",
		"http://localhost:8080/",
		"http://127.0.0.1",
		"http://127.0.0.1:8080",
	}
	for _, origin := range local {
		if !IsLocalOrigin(origin) {
			t.Errorf("IsLocalOrigin(%q) = false, want true", origin)
		}
		if !IsFakeEnabled("fake", origin) {
			t.Errorf("IsFakeEnabled(fake, %q) = false, want true", origin)
		}
		if IsFakeEnabled("", origin) {
			t.Errorf("IsFakeEnabled('', %q) = true, want false", origin)
		}
	}

	nonLocal := []string{
		"",
		"https://localhost:8080",
		"https://127.0.0.1:8080",
		"https://pilot-shortlink-b.onrender.com",
		"http://localhost.evil.test",
		"http://localhost@evil.test",
		"http://127.0.0.2:8080",
		"http://localhost:8080/path",
		"http://example.com",
	}
	for _, origin := range nonLocal {
		if IsLocalOrigin(origin) {
			t.Errorf("IsLocalOrigin(%q) = true, want false", origin)
		}
		if IsFakeEnabled("fake", origin) {
			t.Errorf("IsFakeEnabled(fake, %q) = true, want false", origin)
		}
	}
}

func TestResolveIgnoresFakeModeForNonLocalOrigin(t *testing.T) {
	ps := Resolve(Env{
		AppOrigin:          "https://pilot-shortlink-b.onrender.com",
		Mode:               "fake",
		FakeBase:           "http://127.0.0.1:9411",
		GoogleClientID:     "real-gid",
		GoogleClientSecret: "real-gsec",
		GitHubClientID:     "real-ghid",
		GitHubClientSecret: "real-ghsec",
	})
	g := ps.Get(ProviderGoogle)
	if g == nil || g.AuthorizeURL != GoogleAuthorizeURL || g.ClientID != "real-gid" {
		t.Fatalf("google config on non-local origin = %+v, want real Google endpoints", g)
	}
	gh := ps.Get(ProviderGitHub)
	if gh == nil || gh.AuthorizeURL != GitHubAuthorizeURL || gh.ClientID != "real-ghid" {
		t.Fatalf("github config on non-local origin = %+v, want real GitHub endpoints", gh)
	}
	// Missing credentials on non-local origin must return nil even if OAUTH_MODE=fake.
	empty := Resolve(Env{
		AppOrigin: "https://pilot-shortlink-b.onrender.com",
		Mode:      "fake",
		FakeBase:  "http://127.0.0.1:9411",
	})
	if empty.Get(ProviderGoogle) != nil || empty.Get(ProviderGitHub) != nil {
		t.Fatalf("non-local origin with missing real creds must not fall back to fake")
	}
	// Redaction check.
	if strings.Contains(fmt.Sprint(g), "real-gsec") {
		t.Fatalf("ProviderConfig.String leaked client secret: %s", fmt.Sprint(g))
	}
	if strings.Contains(fmt.Sprintf("%#v", g), "real-gsec") || !strings.Contains(fmt.Sprintf("%#v", g), "[redacted]") {
		t.Fatalf("ProviderConfig.GoString leaked or lacked [redacted]: %#v", g)
	}
}

func TestEnvRedaction(t *testing.T) {
	env := Env{
		AppOrigin:          "https://pilot-shortlink-b.onrender.com",
		Mode:               "",
		FakeBase:           "",
		GoogleClientID:     "gid-public",
		GoogleClientSecret: "google-super-secret-123",
		GitHubClientID:     "ghid-public",
		GitHubClientSecret: "github-super-secret-456",
	}
	for _, formatted := range []string{
		fmt.Sprintf("%+v", env),
		fmt.Sprintf("%#v", env),
		fmt.Sprintf("%+v", &env),
		fmt.Sprintf("%#v", &env),
	} {
		for _, secret := range []string{env.GoogleClientSecret, env.GitHubClientSecret} {
			if strings.Contains(formatted, secret) {
				t.Fatalf("formatted Env leaked %q: %s", secret, formatted)
			}
		}
		if !strings.Contains(formatted, "[redacted]") {
			t.Fatalf("formatted Env missing [redacted]: %s", formatted)
		}
	}
}

func TestResolveFakeModeLocalOrigin(t *testing.T) {
	ps := Resolve(Env{
		AppOrigin: "http://localhost:8080",
		Mode:      "fake",
		FakeBase:  "http://127.0.0.1:9411/",
	})
	g := ps.Get(ProviderGoogle)
	if g == nil || g.AuthorizeURL != "http://127.0.0.1:9411/google/authorize" ||
		g.TokenURL != "http://127.0.0.1:9411/google/token" ||
		g.UserInfoURL != "http://127.0.0.1:9411/google/userinfo" ||
		g.ClientID != FakeClientID || g.ClientSecret != FakeClientSecret {
		t.Fatalf("fake google config = %#v", g)
	}
	gh := ps.Get(ProviderGitHub)
	if gh == nil || gh.AuthorizeURL != "http://127.0.0.1:9411/github/login/oauth/authorize" ||
		gh.TokenURL != "http://127.0.0.1:9411/github/login/oauth/access_token" ||
		gh.UserInfoURL != "http://127.0.0.1:9411/github/user" ||
		gh.EmailsURL != "http://127.0.0.1:9411/github/user/emails" ||
		gh.ClientID != FakeClientID || gh.ClientSecret != FakeClientSecret {
		t.Fatalf("fake github config = %#v", gh)
	}
}

func TestBuildAuthorizeURL(t *testing.T) {
	p := &ProviderConfig{
		Provider:     ProviderGoogle,
		AuthorizeURL: GoogleAuthorizeURL,
		Scope:        GoogleScope,
		ClientID:     "cid-1",
	}
	redirectURI := RedirectURI("https://app.example/", ProviderGoogle)
	if redirectURI != "https://app.example/api/auth/oauth/google/callback" {
		t.Fatalf("redirectURI = %q", redirectURI)
	}
	raw, err := p.BuildAuthorizeURL(redirectURI, "state-123", "challenge-456")
	if err != nil {
		t.Fatal(err)
	}
	u, err := url.Parse(raw)
	if err != nil {
		t.Fatal(err)
	}
	q := u.Query()
	for k, want := range map[string]string{
		"response_type":         "code",
		"client_id":             "cid-1",
		"redirect_uri":          redirectURI,
		"scope":                 GoogleScope,
		"state":                 "state-123",
		"code_challenge":        "challenge-456",
		"code_challenge_method": "S256",
	} {
		if got := q.Get(k); got != want {
			t.Errorf("query %s = %q, want %q", k, got, want)
		}
	}
}

func TestSelectGitHubEmail(t *testing.T) {
	cases := []struct {
		name   string
		emails []GitHubEmail
		want   string
		ok     bool
	}{
		{
			name: "picks primary and verified",
			emails: []GitHubEmail{
				{Email: "secondary@example.test", Primary: false, Verified: true},
				{Email: "Primary@Example.Test", Primary: true, Verified: true},
				{Email: "unverified@example.test", Primary: false, Verified: false},
			},
			want: "primary@example.test",
			ok:   true,
		},
		{
			name: "rejects primary unverified even if secondary is verified",
			emails: []GitHubEmail{
				{Email: "secondary@example.test", Primary: false, Verified: true},
				{Email: "primary@example.test", Primary: true, Verified: false},
			},
			want: "",
			ok:   false,
		},
		{
			name:   "rejects empty list",
			emails: nil,
			want:   "",
			ok:     false,
		},
		{
			name: "rejects malformed primary verified email",
			emails: []GitHubEmail{
				{Email: "not-an-email", Primary: true, Verified: true},
			},
			want: "",
			ok:   false,
		},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			got, ok := SelectGitHubEmail(tc.emails)
			if got != tc.want || ok != tc.ok {
				t.Fatalf("SelectGitHubEmail() = (%q, %v), want (%q, %v)", got, ok, tc.want, tc.ok)
			}
		})
	}
}

func TestExchangeAndIdentityHeadersAndRedaction(t *testing.T) {
	var gotAccept, gotUA, gotGrant, gotVerifier string
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Path {
		case "/token":
			gotAccept = r.Header.Get("Accept")
			gotUA = r.Header.Get("User-Agent")
			_ = r.ParseForm()
			gotGrant = r.Form.Get("grant_type")
			gotVerifier = r.Form.Get("code_verifier")
			w.Header().Set("Content-Type", "application/json")
			_, _ = w.Write([]byte(`{"access_token":"tok-secret-xyz"}`))
		case "/user":
			if r.Header.Get("Accept") != "application/vnd.github+json" || r.Header.Get("User-Agent") == "" {
				w.WriteHeader(http.StatusBadRequest)
				return
			}
			_, _ = w.Write([]byte(`{"id":4242}`))
		case "/user/emails":
			_, _ = w.Write([]byte(`[{"email":"GH@example.test","primary":true,"verified":true}]`))
		case "/fail":
			w.WriteHeader(http.StatusInternalServerError)
			_, _ = w.Write([]byte(`{"error":"secret-body-leak"}`))
		}
	}))
	defer srv.Close()

	p := &ProviderConfig{
		Provider:     ProviderGitHub,
		TokenURL:     srv.URL + "/token",
		UserInfoURL:  srv.URL + "/user",
		EmailsURL:    srv.URL + "/user/emails",
		ClientID:     "cid",
		ClientSecret: "csec",
	}
	tok, err := p.Exchange(context.Background(), "code-1", "http://localhost:8080/api/auth/oauth/github/callback", "verifier-1")
	if err != nil || tok != "tok-secret-xyz" {
		t.Fatalf("Exchange = (%q, %v)", tok, err)
	}
	if gotAccept != "application/json" || gotUA == "" || gotGrant != "authorization_code" || gotVerifier != "verifier-1" {
		t.Fatalf("token request mismatch: accept=%q ua=%q grant=%q verifier=%q", gotAccept, gotUA, gotGrant, gotVerifier)
	}
	id, err := p.Identity(context.Background(), tok)
	if err != nil {
		t.Fatalf("Identity: %v", err)
	}
	if id.Subject != "4242" || id.Email != "gh@example.test" || !id.EmailVerified {
		t.Fatalf("Identity = %+v", id)
	}

	p.TokenURL = srv.URL + "/fail"
	_, err = p.Exchange(context.Background(), "secret-code", "http://localhost/cb", "secret-verifier")
	if err == nil {
		t.Fatal("expected error on 500 token response")
	}
	for _, secret := range []string{"secret-body-leak", "secret-code", "secret-verifier", "csec"} {
		if strings.Contains(err.Error(), secret) {
			t.Fatalf("Exchange error leaked %q: %v", secret, err)
		}
	}
}
