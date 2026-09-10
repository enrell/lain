// Package webui embeds the compiled frontend and serves it as a
// single-page application from the existing gateway mux.
//
// The frontend lives in web/ (SvelteKit, static adapter) and is built
// into internal/webui/dist by the project build (see Makefile and the
// Dockerfile). No Node runtime is needed here: the dist tree is plain
// static files, embedded at compile time and served by the same Go
// process that owns /api and media streaming.
package webui

import (
	"bytes"
	"embed"
	"io"
	"io/fs"
	"mime"
	"net/http"
	"path"
	"strings"
	"time"
)

//go:embed all:dist
var embedded embed.FS

// FS returns the compiled frontend filesystem rooted at dist.
func FS() (fs.FS, error) { return fs.Sub(embedded, "dist") }

// Mount registers the SPA handler on mux. Existing method patterns
// (API, health, media routes) are more specific and keep winning;
// everything else — including unknown /api paths — falls through to
// this handler, which never lets an API request become HTML.
func Mount(mux *http.ServeMux) {
	sub, err := FS()
	if err != nil {
		mux.Handle("/", http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
			http.Error(w, "web UI unavailable: "+err.Error(), http.StatusInternalServerError)
		}))
		return
	}
	mux.Handle("/", Handler(sub))
}

const (
	indexFile       = "index.html"
	immutablePrefix = "_app/immutable/"
)

// Handler serves one static frontend tree. It is exported separately
// for tests and for callers that want to mount a different build.
//
// Semantics:
//   - GET/HEAD only.
//   - /api/* and /health* never receive HTML (JSON 404 when unknown).
//   - existing files are served with explicit content types;
//     hashed build assets get immutable caching, other assets 1h.
//   - index.html is served for client-side routes and never cached hard.
//   - asset-looking missing paths get a real 404, not the SPA shell.
//   - path traversal is rejected before any filesystem access.
func Handler(fsys fs.FS) http.Handler {
	index, indexErr := fs.ReadFile(fsys, indexFile)
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.Method != http.MethodGet && r.Method != http.MethodHead {
			w.Header().Set("Allow", "GET, HEAD")
			writeJSONError(w, http.StatusMethodNotAllowed, "method not allowed")
			return
		}
		w.Header().Set("X-Content-Type-Options", "nosniff")

		p := r.URL.Path
		if isAPIPath(p) {
			writeJSONError(w, http.StatusNotFound, "not found")
			return
		}
		if strings.Contains(p, "..") || strings.ContainsRune(p, 0) {
			writeJSONError(w, http.StatusBadRequest, "bad path")
			return
		}
		if p == "" || p == "/" {
			serveIndex(w, r, index, indexErr)
			return
		}
		name := strings.TrimPrefix(path.Clean("/"+p), "/")
		if name == "" || name == "." {
			serveIndex(w, r, index, indexErr)
			return
		}
		if f, err := fsys.Open(name); err == nil {
			defer f.Close()
			if st, err := f.Stat(); err == nil && !st.IsDir() {
				serveAsset(w, r, name, f, st)
				return
			}
		}
		if isAssetPath(name) {
			writeJSONError(w, http.StatusNotFound, "not found")
			return
		}
		serveIndex(w, r, index, indexErr)
	})
}

func isAPIPath(p string) bool {
	return p == "/api" || strings.HasPrefix(p, "/api/") ||
		p == "/health" || strings.HasPrefix(p, "/health/")
}

// isAssetPath decides whether a missing path looks like a build asset
// (real 404) or a client-side route (SPA fallback).
func isAssetPath(name string) bool {
	if strings.HasPrefix(name, "_app/") {
		return true
	}
	switch strings.ToLower(path.Ext(name)) {
	case ".js", ".mjs", ".css", ".map", ".json", ".txt", ".xml", ".webmanifest",
		".png", ".jpg", ".jpeg", ".gif", ".svg", ".webp", ".avif", ".ico",
		".woff", ".woff2", ".ttf", ".otf", ".wasm", ".mp3", ".mp4", ".webm":
		return true
	}
	return false
}

func serveIndex(w http.ResponseWriter, r *http.Request, index []byte, indexErr error) {
	w.Header().Set("Content-Type", "text/html; charset=utf-8")
	if indexErr != nil {
		w.WriteHeader(http.StatusServiceUnavailable)
		_, _ = io.WriteString(w, notBuiltNotice)
		return
	}
	w.Header().Set("Cache-Control", "no-cache")
	http.ServeContent(w, r, indexFile, time.Time{}, bytes.NewReader(index))
}

func serveAsset(w http.ResponseWriter, r *http.Request, name string, f fs.File, st fs.FileInfo) {
	w.Header().Set("Content-Type", contentType(name))
	if strings.HasPrefix(name, immutablePrefix) {
		w.Header().Set("Cache-Control", "public, max-age=31536000, immutable")
	} else {
		w.Header().Set("Cache-Control", "public, max-age=3600")
	}
	rs, ok := readSeeker(f, st)
	if !ok {
		raw, err := io.ReadAll(f)
		if err != nil {
			writeJSONError(w, http.StatusInternalServerError, "read failed")
			return
		}
		rs = bytes.NewReader(raw)
	}
	http.ServeContent(w, r, name, st.ModTime(), rs)
}

// readSeeker avoids copying a whole asset into the heap per request:
// embed files and fstest files expose ReaderAt, which SectionReader
// turns into the ReadSeeker ServeContent needs for ranges.
func readSeeker(f fs.File, st fs.FileInfo) (io.ReadSeeker, bool) {
	if rs, ok := f.(io.ReadSeeker); ok {
		return rs, true
	}
	if ra, ok := f.(io.ReaderAt); ok {
		return io.NewSectionReader(ra, 0, st.Size()), true
	}
	return nil, false
}

func contentType(name string) string {
	if ct := mime.TypeByExtension(path.Ext(name)); ct != "" {
		return ct
	}
	switch strings.ToLower(path.Ext(name)) {
	case ".js", ".mjs":
		return "text/javascript; charset=utf-8"
	case ".css":
		return "text/css; charset=utf-8"
	case ".svg":
		return "image/svg+xml"
	case ".woff2":
		return "font/woff2"
	case ".woff":
		return "font/woff"
	case ".webmanifest":
		return "application/manifest+json"
	case ".wasm":
		return "application/wasm"
	}
	return "application/octet-stream"
}

func writeJSONError(w http.ResponseWriter, code int, msg string) {
	w.Header().Set("Content-Type", "application/json")
	w.WriteHeader(code)
	_, _ = io.WriteString(w, `{"error":"`+msg+`"}`+"\n")
}

// notBuiltNotice is what a binary built without the frontend serves.
// Plain, honest, and useful for development.
const notBuiltNotice = `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Lain — web UI not built</title>
<style>
  body { margin: 0; min-height: 100vh; display: grid; place-items: center;
         background: #0a0b0e; color: #e8eaf0;
         font: 15px/1.6 system-ui, sans-serif; }
  main { max-width: 34rem; padding: 2rem; }
  h1 { font-size: 1.1rem; letter-spacing: .04em; text-transform: uppercase; color: #8b92a5; }
  code { background: #12141a; border: 1px solid #232732; border-radius: 6px;
         padding: .15rem .4rem; font-size: .9em; color: #6ee7ff; }
</style>
</head>
<body>
<main>
<h1>Lain</h1>
<p>The API is running, but this binary was built without the web UI.</p>
<p>Build it with <code>make web</code> (needs pnpm once), or develop against
the Vite dev server: <code>cd web &amp;&amp; pnpm dev</code>.</p>
</main>
</body>
</html>
`
