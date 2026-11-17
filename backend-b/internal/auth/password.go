// Package auth hashes passwords (Argon2id, ADR-002) and makes session tokens.
package auth

import (
	"crypto/rand"
	"crypto/sha256"
	"crypto/subtle"
	"encoding/base64"
	"encoding/hex"
	"errors"
	"fmt"
	"strings"

	"golang.org/x/crypto/argon2"
)

// OWASP minimum for Argon2id: m=19 MiB, t=2, p=1 (docs/architecture.md ADR-002).
const (
	memoryKiB = 19456
	timeCost  = 2
	threads   = 1
	keyLen    = 32
	saltLen   = 16
)

var b64 = base64.RawStdEncoding

// HashPassword returns a PHC string: $argon2id$v=19$m=19456,t=2,p=1$<salt>$<hash>.
func HashPassword(password string) (string, error) {
	salt := make([]byte, saltLen)
	if _, err := rand.Read(salt); err != nil {
		return "", err
	}
	key := argon2.IDKey([]byte(password), salt, timeCost, memoryKiB, threads, keyLen)
	return fmt.Sprintf("$argon2id$v=%d$m=%d,t=%d,p=%d$%s$%s",
		argon2.Version, memoryKiB, timeCost, threads, b64.EncodeToString(salt), b64.EncodeToString(key)), nil
}

// VerifyPassword compares in constant time using the parameters stored in the PHC string.
func VerifyPassword(password, phc string) (bool, error) {
	parts := strings.Split(phc, "$")
	if len(parts) != 6 || parts[1] != "argon2id" {
		return false, errors.New("not an argon2id PHC string")
	}
	var version int
	var m, t uint32
	var p uint8
	if _, err := fmt.Sscanf(parts[2], "v=%d", &version); err != nil || version != argon2.Version {
		return false, errors.New("unsupported argon2 version")
	}
	if _, err := fmt.Sscanf(parts[3], "m=%d,t=%d,p=%d", &m, &t, &p); err != nil {
		return false, fmt.Errorf("argon2 params: %w", err)
	}
	salt, err := b64.DecodeString(parts[4])
	if err != nil {
		return false, err
	}
	want, err := b64.DecodeString(parts[5])
	if err != nil {
		return false, err
	}
	got := argon2.IDKey([]byte(password), salt, t, m, p, uint32(len(want)))
	return subtle.ConstantTimeCompare(got, want) == 1, nil
}

// dummyHash is verified against when the email does not exist, so a wrong email
// takes about as long as a wrong password (docs/architecture.md section 6).
var dummyHash = func() string {
	h, err := HashPassword("dummy-password-never-matches")
	if err != nil {
		panic(err)
	}
	return h
}()

// BurnTime spends the same Argon2 cost as a real check.
func BurnTime(password string) { _, _ = VerifyPassword(password, dummyHash) }

// NewSessionToken returns a 32-byte random token (base64url, no padding: 43 chars) and its SHA-256 hex.
// The cookie gets the token; the database stores only the hash.
func NewSessionToken() (token, tokenHash string, err error) {
	b := make([]byte, 32)
	if _, err = rand.Read(b); err != nil {
		return "", "", err
	}
	token = base64.RawURLEncoding.EncodeToString(b)
	return token, HashToken(token), nil
}

// HashToken is the database key for a session token.
func HashToken(token string) string {
	sum := sha256.Sum256([]byte(token))
	return hex.EncodeToString(sum[:])
}
