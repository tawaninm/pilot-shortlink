// Package site serves the built web app (web/dist) from backend B on the same origin as the API (ADR-001).
// scripts/verify.py and the Dockerfile copy web/dist into ./dist before building; Go's embed cannot read
// outside the module. dist/.keep is committed so the package builds before the first copy.
package site

import (
	"embed"
	"io/fs"
	"net/http"
	"strings"
)

//go:embed all:dist
var files embed.FS

// CSP for the app shell: own scripts only, Google Fonts for type (DESIGN.md), no framing (OWASP A02).
const CSP = "default-src 'self'; script-src 'self'; style-src 'self' https://fonts.googleapis.com; " +
	"font-src https://fonts.gstatic.com; img-src 'self' data:; connect-src 'self'; " +
	"frame-ancestors 'none'; base-uri 'self'; form-action 'self'"

// HTMLHeaders sets the headers every HTML page gets.
func HTMLHeaders(h http.Header) {
	h.Set("Content-Security-Policy", CSP)
	h.Set("Referrer-Policy", "strict-origin-when-cross-origin")
	h.Set("Strict-Transport-Security", "max-age=31536000; includeSubDomains")
}

// Site holds the app shell and the hashed assets.
type Site struct {
	index  []byte
	assets fs.FS
}

// New loads the embedded build. A missing index.html (no build copied in) makes Index answer 503.
func New() *Site {
	dist, _ := fs.Sub(files, "dist")
	index, _ := fs.ReadFile(dist, "index.html")
	assets, _ := fs.Sub(dist, "assets")
	return &Site{index: index, assets: assets}
}

// Index serves the single-page app shell for the app's own routes (/, /login, /register).
func (s *Site) Index(w http.ResponseWriter, _ *http.Request) {
	if len(s.index) == 0 {
		http.Error(w, "web app not built", http.StatusServiceUnavailable)
		return
	}
	HTMLHeaders(w.Header())
	w.Header().Set("Content-Type", "text/html; charset=utf-8")
	w.Header().Set("Cache-Control", "no-cache")
	_, _ = w.Write(s.index)
}

// Assets serves /assets/<file>. File names carry a content hash, so they can be cached for a year.
// Directory paths are 404 rather than a listing.
func (s *Site) Assets() http.Handler {
	files := http.StripPrefix("/assets/", http.FileServerFS(s.assets))
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		name := strings.TrimPrefix(r.URL.Path, "/assets/")
		if name == "" || strings.HasSuffix(name, "/") {
			http.NotFound(w, r)
			return
		}
		if _, err := fs.Stat(s.assets, name); err != nil {
			http.NotFound(w, r)
			return
		}
		w.Header().Set("Cache-Control", "public, max-age=31536000, immutable")
		files.ServeHTTP(w, r)
	})
}
