package api

import (
	"crypto/rand"
	"net/url"
	"regexp"
	"strings"
)

// Rules from docs/requirements.md section 4 and api/openapi.yaml (TargetUrl, CreateLink.alias).

const maxURLLen = 2048

var (
	aliasPattern = regexp.MustCompile(`^[a-z0-9](?:[a-z0-9-]{1,30}[a-z0-9])?$`)
	reserved     = map[string]bool{
		"api": true, "app": true, "admin": true, "login": true, "logout": true, "register": true,
		"static": true, "assets": true, "health": true, "openapi": true,
		"verify-email": true, "forgot-password": true, "reset-password": true,
	}
)

// checkTargetURL returns "" when raw is a usable target, else the Problem code (invalid_url or self_link).
func checkTargetURL(raw, appOrigin string) string {
	if len(raw) > maxURLLen {
		return "invalid_url"
	}
	u, err := url.Parse(raw)
	if err != nil || (u.Scheme != "http" && u.Scheme != "https") || u.Hostname() == "" {
		return "invalid_url"
	}
	// "host." is the same host to a browser, so compare without the trailing dot (S7 security review).
	host, appHost := strings.TrimSuffix(u.Hostname(), "."), ""
	if app, err := url.Parse(appOrigin); err == nil {
		appHost = strings.TrimSuffix(app.Hostname(), ".")
	}
	if appHost != "" && strings.EqualFold(host, appHost) {
		return "self_link" // would redirect into the app itself (loops, phishing under our name)
	}
	return ""
}

// checkAlias lower-cases the alias and returns it with "" or the Problem code (invalid_alias or reserved_alias).
func checkAlias(raw string) (string, string) {
	// Check ASCII before lowercasing: Unicode folding turns e.g. the Kelvin sign into "k" (S7 security review).
	for i := 0; i < len(raw); i++ {
		if raw[i] >= 0x80 {
			return raw, "invalid_alias"
		}
	}
	alias := strings.ToLower(raw)
	if len(alias) < 3 || !aliasPattern.MatchString(alias) {
		return alias, "invalid_alias"
	}
	if reserved[alias] {
		return alias, "reserved_alias"
	}
	return alias, ""
}

const codeAlphabet = "abcdefghijklmnopqrstuvwxyz0123456789"

// randomCode returns 7 characters from [a-z0-9] using crypto/rand. Bytes >= 252 are skipped so
// every character is equally likely (252 = 7 * 36; no modulo bias).
func randomCode() (string, error) {
	out := make([]byte, 0, 7)
	buf := make([]byte, 16)
	for len(out) < 7 {
		if _, err := rand.Read(buf); err != nil {
			return "", err
		}
		for _, b := range buf {
			if b < 252 && len(out) < 7 {
				out = append(out, codeAlphabet[int(b)%36])
			}
		}
	}
	return string(out), nil
}

// referrerHost keeps only the lower-cased host of a Referer header ("" when absent or unparsable).
// Paths and query strings are dropped: they can carry personal data (requirements section 4).
func referrerHost(referer string) string {
	u, err := url.Parse(referer)
	if err != nil {
		return ""
	}
	return strings.ToLower(u.Hostname())
}
