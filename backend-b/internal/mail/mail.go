// Package mail defines the outbound email port and its adapters (Resend for production, Capture for contract/e2e tests).
package mail

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"html"
	"io"
	"log/slog"
	"net/http"
	"os"
	"strings"
	"sync"
	"time"
)

const (
	PurposeVerify    = "verify"
	PurposeReset     = "reset"
	DefaultFrom      = "pilot-shortlink <onboarding@resend.dev>"
	DefaultResendURL = "https://api.resend.com/emails"
	resendTimeout    = 10 * time.Second
)

// ErrMissingAPIKey is returned when Resend is selected without RESEND_API_KEY set.
var ErrMissingAPIKey = errors.New("resend: missing RESEND_API_KEY")

// Mail is a single outbound message. IdempotencyKey is sent to the provider and omitted from capture JSON.
type Mail struct {
	To             string `json:"to"`
	Subject        string `json:"subject"`
	Text           string `json:"text"`
	HTML           string `json:"html"`
	IdempotencyKey string `json:"-"`
}

// Mailer is the outbound email port.
type Mailer interface {
	Send(ctx context.Context, m Mail) error
}

// Build constructs the verification or password-reset message using appOrigin (never request headers, A05).
func Build(purpose, to, appOrigin, token, tokenHash string) Mail {
	subject := "ยืนยันอีเมลสำหรับ pilot-shortlink"
	path := "/verify-email?token="
	if purpose == PurposeReset {
		subject = "ตั้งรหัสผ่านใหม่สำหรับ pilot-shortlink"
		path = "/reset-password?token="
	}
	link := strings.TrimRight(appOrigin, "/") + path + token
	escaped := html.EscapeString(link)
	prefix := tokenHash
	if len(prefix) > 16 {
		prefix = prefix[:16]
	}
	return Mail{
		To:             to,
		Subject:        subject,
		Text:           subject + " (ลิงก์ใช้ได้ 30 นาที):\n" + link,
		HTML:           "<p>" + subject + " (ลิงก์ใช้ได้ 30 นาที):</p><p><a href=\"" + escaped + "\">" + escaped + "</a></p>",
		IdempotencyKey: purpose + "/" + prefix,
	}
}

// Capture stores sent emails in memory when MAIL_MODE == "capture".
type Capture struct {
	mu    sync.Mutex
	mails []Mail
}

// NewCapture returns an empty in-memory mail store.
func NewCapture() *Capture {
	return &Capture{}
}

// Send appends m to the in-memory store.
func (c *Capture) Send(_ context.Context, m Mail) error {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.mails = append(c.mails, m)
	return nil
}

// For returns all captured emails addressed to recipient (oldest first).
func (c *Capture) For(to string) []Mail {
	c.mu.Lock()
	defer c.mu.Unlock()
	out := make([]Mail, 0)
	for _, m := range c.mails {
		if strings.EqualFold(m.To, to) {
			out = append(out, m)
		}
	}
	return out
}

// Resend sends email via Resend's HTTP API.
type Resend struct {
	APIKey string
	From   string
	URL    string
	Client *http.Client
}

// LogValue redacts APIKey if Resend is ever passed to slog.
func (r Resend) LogValue() slog.Value {
	return slog.GroupValue(
		slog.String("api_key", "[redacted]"),
		slog.String("from", r.From),
		slog.String("url", r.URL),
	)
}

// String redacts APIKey when formatted via %v / %+v.
func (r Resend) String() string {
	return fmt.Sprintf("mail.Resend{APIKey:%q From:%q URL:%q}", "[redacted]", r.From, r.URL)
}

// GoString redacts APIKey when formatted via %#v.
func (r Resend) GoString() string {
	return fmt.Sprintf("mail.Resend{APIKey:%q, From:%q, URL:%q}", "[redacted]", r.From, r.URL)
}

// NewResend configures a Resend adapter with a 10 s timeout and the default sender fallback.
func NewResend(apiKey, from string) *Resend {
	if from == "" {
		from = DefaultFrom
	}
	return &Resend{
		APIKey: apiKey,
		From:   from,
		URL:    DefaultResendURL,
		Client: &http.Client{Timeout: resendTimeout},
	}
}

type resendRequest struct {
	From    string   `json:"from"`
	To      []string `json:"to"`
	Subject string   `json:"subject"`
	Text    string   `json:"text"`
	HTML    string   `json:"html"`
}

// Send posts the email to Resend. When APIKey is empty it skips the HTTP call and returns ErrMissingAPIKey.
func (r *Resend) Send(ctx context.Context, m Mail) error {
	if r.APIKey == "" {
		return ErrMissingAPIKey
	}
	from := r.From
	if from == "" {
		from = DefaultFrom
	}
	endpoint := r.URL
	if endpoint == "" {
		endpoint = DefaultResendURL
	}
	client := r.Client
	if client == nil {
		client = &http.Client{Timeout: resendTimeout}
	}

	body, err := json.Marshal(resendRequest{
		From:    from,
		To:      []string{m.To},
		Subject: m.Subject,
		Text:    m.Text,
		HTML:    m.HTML,
	})
	if err != nil {
		return err
	}
	req, err := http.NewRequestWithContext(ctx, http.MethodPost, endpoint, bytes.NewReader(body))
	if err != nil {
		return err
	}
	req.Header.Set("Authorization", "Bearer "+r.APIKey)
	req.Header.Set("Content-Type", "application/json")
	if m.IdempotencyKey != "" {
		req.Header.Set("Idempotency-Key", m.IdempotencyKey)
	}

	resp, err := client.Do(req)
	if err != nil {
		return errors.New("resend: request failed")
	}
	defer resp.Body.Close()
	_, _ = io.Copy(io.Discard, io.LimitReader(resp.Body, 4096))
	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		return fmt.Errorf("resend: status %d", resp.StatusCode)
	}
	return nil
}

// FromEnv chooses Capture when MAIL_MODE == "capture" and Resend otherwise.
// The returned *Capture is non-nil only in capture mode (AC-V8).
func FromEnv() (Mailer, *Capture) {
	if os.Getenv("MAIL_MODE") == "capture" {
		c := NewCapture()
		return c, c
	}
	return NewResend(os.Getenv("RESEND_API_KEY"), os.Getenv("MAIL_FROM")), nil
}
