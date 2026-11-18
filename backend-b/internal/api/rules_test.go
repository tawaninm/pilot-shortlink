package api

import (
	"regexp"
	"strings"
	"testing"
)

const app = "https://short.example"

func TestCheckTargetURL(t *testing.T) {
	cases := map[string]string{
		"https://example.com/a?b=c": "",
		"http://example.com":        "",
		"javascript:alert(1)":       "invalid_url",
		"data:text/html,<script>":   "invalid_url",
		"ftp://example.com/a":       "invalid_url",
		"not a url":                 "invalid_url",
		"https://":                  "invalid_url",
		"https://example.com/" + strings.Repeat("a", 2048): "invalid_url",
		"https://short.example/abc1234":                    "self_link",
		"https://SHORT.example:8443/x":                     "self_link",
		"https://short.example./x":                         "self_link",
	}
	for in, want := range cases {
		if got := checkTargetURL(in, app); got != want {
			t.Errorf("checkTargetURL(%.40q) = %q, want %q", in, got, want)
		}
	}
}

func TestCheckAlias(t *testing.T) {
	cases := map[string][2]string{
		"Tawan-CV":              {"tawan-cv", ""},
		"abc":                   {"abc", ""},
		"ab":                    {"ab", "invalid_alias"},
		"-cv":                   {"-cv", "invalid_alias"},
		"cv-":                   {"cv-", "invalid_alias"},
		"a_b_c":                 {"a_b_c", "invalid_alias"},
		strings.Repeat("x", 33): {strings.Repeat("x", 33), "invalid_alias"},
		"API":                   {"api", "reserved_alias"},
		"Health":                {"health", "reserved_alias"},
		"Kelvin":                {"Kelvin", "invalid_alias"},
	}
	for in, want := range cases {
		alias, code := checkAlias(in)
		if alias != want[0] || code != want[1] {
			t.Errorf("checkAlias(%q) = %q, %q; want %q, %q", in, alias, code, want[0], want[1])
		}
	}
}

func TestRandomCode(t *testing.T) {
	re := regexp.MustCompile(`^[a-z0-9]{7}$`)
	seen := map[string]bool{}
	for range 1000 {
		c, err := randomCode()
		if err != nil || !re.MatchString(c) {
			t.Fatalf("randomCode() = %q, %v", c, err)
		}
		if seen[c] {
			t.Fatalf("repeat after %d codes: %s", len(seen), c)
		}
		seen[c] = true
	}
}

func TestReferrerHost(t *testing.T) {
	cases := map[string]string{
		"https://www.Facebook.com/x?fbclid=abc": "www.facebook.com",
		"":                                      "",
		"::not a url":                           "",
		"https://user:pw@example.org:8080/p":    "example.org",
	}
	for in, want := range cases {
		if got := referrerHost(in); got != want {
			t.Errorf("referrerHost(%q) = %q, want %q", in, got, want)
		}
	}
}
