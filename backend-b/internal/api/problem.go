package api

import (
	"encoding/json"
	"net/http"
	"strconv"

	"github.com/tawaninm/pilot-shortlink/backend-b/internal/site"
)

// problem is an RFC 9457 body. Code is one of Problem.code in api/openapi.yaml.
type problem struct {
	Type   string `json:"type"`
	Title  string `json:"title"`
	Status int    `json:"status"`
	Code   string `json:"code"`
	Field  string `json:"field,omitempty"`
	Detail string `json:"detail,omitempty"`
}

func writeJSON(w http.ResponseWriter, status int, v any) {
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(status)
	_ = json.NewEncoder(w).Encode(v)
}

// writeProblem never includes internal error text: callers log the cause and send only the code.
func writeProblem(w http.ResponseWriter, status int, code, field string) {
	w.Header().Set("Content-Type", "application/problem+json")
	w.WriteHeader(status)
	_ = json.NewEncoder(w).Encode(problem{
		Type: "about:blank", Title: http.StatusText(status), Status: status, Code: code, Field: field,
	})
}

// writeRateLimited answers 429 with Retry-After in whole seconds (at least 1).
func writeRateLimited(w http.ResponseWriter, retryAfterSeconds int) {
	w.Header().Set("Retry-After", strconv.Itoa(max(1, retryAfterSeconds)))
	writeProblem(w, http.StatusTooManyRequests, "rate_limited", "")
}

const notFoundHTML = `<!doctype html><html lang="th"><meta charset="utf-8"><title>ไม่พบลิงก์นี้</title><h1>ไม่พบลิงก์นี้</h1><p>ลิงก์อาจถูกลบหรือพิมพ์ผิด</p></html>`

func writeNotFoundPage(w http.ResponseWriter) {
	site.HTMLHeaders(w.Header())
	w.Header().Set("Content-Type", "text/html; charset=utf-8")
	w.Header().Set("Cache-Control", "no-store")
	w.WriteHeader(http.StatusNotFound)
	_, _ = w.Write([]byte(notFoundHTML))
}
