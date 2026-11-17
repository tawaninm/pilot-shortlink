package auth

import (
	"regexp"
	"strings"
	"testing"
)

func TestPasswordRoundTrip(t *testing.T) {
	h, err := HashPassword("correct-horse-battery")
	if err != nil {
		t.Fatal(err)
	}
	if !strings.HasPrefix(h, "$argon2id$v=19$m=19456,t=2,p=1$") {
		t.Fatalf("PHC params: %s", h)
	}
	if ok, err := VerifyPassword("correct-horse-battery", h); !ok || err != nil {
		t.Fatalf("right password rejected: %v", err)
	}
	if ok, _ := VerifyPassword("wrong-password-x", h); ok {
		t.Fatal("wrong password accepted")
	}
	h2, _ := HashPassword("correct-horse-battery")
	if h == h2 {
		t.Fatal("same salt used twice")
	}
}

func TestVerifyRejectsMalformed(t *testing.T) {
	for _, bad := range []string{"", "plain", "$argon2i$v=19$m=1,t=1,p=1$AA$AA", "$argon2id$v=19$m=x$AA$AA"} {
		if ok, err := VerifyPassword("x", bad); ok || err == nil {
			t.Errorf("%q: ok=%v err=%v", bad, ok, err)
		}
	}
}

func TestSessionToken(t *testing.T) {
	tok, hash, err := NewSessionToken()
	if err != nil {
		t.Fatal(err)
	}
	if !regexp.MustCompile(`^[A-Za-z0-9_-]{43}$`).MatchString(tok) {
		t.Fatalf("token format: %q", tok)
	}
	if hash != HashToken(tok) || len(hash) != 64 || strings.Contains(hash, tok) {
		t.Fatalf("hash: %q", hash)
	}
	tok2, _, _ := NewSessionToken()
	if tok == tok2 {
		t.Fatal("tokens repeat")
	}
}
