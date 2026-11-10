//! The web SPA (D-235): the SvelteKit static build, embedded into the
//! `lain` binary at compile time and served from the same origin as
//! `/api` and the media streams, with the rules of Go's `internal/webui`:
//!
//! - GET and HEAD only (others: 405 JSON with `Allow: GET, HEAD`);
//! - `/api/*` and `/health*` never receive HTML (JSON 404 when unknown);
//! - a path with `..` or NUL (decoded) is a 400 before any lookup;
//! - files carry explicit content types and `nosniff`; hashed build
//!   assets (`_app/immutable/`) are cached for a year, other files 1 h;
//! - `index.html` answers client-side routes with `no-cache`;
//! - a missing path that looks like an asset is a real 404, not the shell;
//! - a build without the web UI serves a 503 notice page.
//!
//! The server crate holds no files itself: the binary hands it a table
//! of `(path, bytes)` (see `crates/lain/build.rs`), so tests can mount
//! their own tree and a web rebuild only relinks the binary.

use axum::body::Body;
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::response::Response;

use crate::serve::{RangeError, parse_range};

/// An embedded static tree: relative paths (`/`-separated) and bytes.
#[derive(Clone, Copy, Default)]
pub struct WebAssets {
    files: &'static [(&'static str, &'static [u8])],
}

impl std::fmt::Debug for WebAssets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "WebAssets({} files)", self.files.len())
    }
}

const INDEX: &str = "index.html";
const IMMUTABLE: &str = "_app/immutable/";

impl WebAssets {
    pub const fn new(files: &'static [(&'static str, &'static [u8])]) -> Self {
        WebAssets { files }
    }

    /// No web UI: every SPA path answers the build notice.
    pub const fn none() -> Self {
        WebAssets { files: &[] }
    }

    fn get(&self, name: &str) -> Option<&'static [u8]> {
        self.files.iter().find(|(n, _)| *n == name).map(|(_, b)| *b)
    }

    /// Whether the tree has a shell to serve.
    pub fn is_built(&self) -> bool {
        self.get(INDEX).is_some()
    }

    /// The answer for a request the API router did not take.
    pub fn respond(&self, method: &Method, raw_path: &str, req: &HeaderMap) -> Response {
        if method != Method::GET && method != Method::HEAD {
            let mut r = json_error(StatusCode::METHOD_NOT_ALLOWED, "method not allowed");
            r.headers_mut().insert(header::ALLOW, HeaderValue::from_static("GET, HEAD"));
            return r;
        }
        let Some(path) = percent_decode(raw_path) else {
            return nosniff(json_error(StatusCode::BAD_REQUEST, "bad path"));
        };
        if is_api_path(&path) {
            return nosniff(json_error(StatusCode::NOT_FOUND, "not found"));
        }
        if path.contains("..") || path.contains('\0') {
            return nosniff(json_error(StatusCode::BAD_REQUEST, "bad path"));
        }
        let name = clean(&path);
        if name.is_empty() {
            return self.index(method, req);
        }
        if let Some(bytes) = self.get(&name) {
            let cache = if name.starts_with(IMMUTABLE) {
                "public, max-age=31536000, immutable"
            } else {
                "public, max-age=3600"
            };
            return content(method, req, content_type(&name), cache, bytes);
        }
        // A directory lookup never lists: it falls through like a route.
        if is_asset_path(&name) {
            return nosniff(json_error(StatusCode::NOT_FOUND, "not found"));
        }
        self.index(method, req)
    }

    fn index(&self, method: &Method, req: &HeaderMap) -> Response {
        match self.get(INDEX) {
            Some(bytes) => content(method, req, "text/html; charset=utf-8", "no-cache", bytes),
            None => {
                let mut r = Response::new(Body::from(NOT_BUILT));
                *r.status_mut() = StatusCode::SERVICE_UNAVAILABLE;
                r.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("text/html; charset=utf-8"));
                nosniff(r)
            }
        }
    }
}

/// `/api`, `/api/...`, `/health` and `/health/...`.
pub fn is_api_path(p: &str) -> bool {
    p == "/api" || p.starts_with("/api/") || p == "/health" || p.starts_with("/health/")
}

/// Whether a missing path looks like a build asset (a real 404) rather
/// than a client-side route (the SPA shell).
fn is_asset_path(name: &str) -> bool {
    if name.starts_with("_app/") {
        return true;
    }
    let ext = extension(name).to_ascii_lowercase();
    matches!(
        ext.as_str(),
        "js" | "mjs"
            | "css"
            | "map"
            | "json"
            | "txt"
            | "xml"
            | "webmanifest"
            | "png"
            | "jpg"
            | "jpeg"
            | "gif"
            | "svg"
            | "webp"
            | "avif"
            | "ico"
            | "woff"
            | "woff2"
            | "ttf"
            | "otf"
            | "wasm"
            | "mp3"
            | "mp4"
            | "webm"
    )
}

fn extension(name: &str) -> &str {
    let base = name.rsplit('/').next().unwrap_or(name);
    match base.rfind('.') {
        Some(i) => &base[i + 1..],
        None => "",
    }
}

/// Go's `mime.TypeByExtension` built-ins plus the fallbacks `webui` added.
fn content_type(name: &str) -> &'static str {
    match extension(name).to_ascii_lowercase().as_str() {
        "html" | "htm" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "map" => "application/json",
        "xml" => "text/xml; charset=utf-8",
        "txt" => "text/plain; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "ico" => "image/vnd.microsoft.icon",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "webmanifest" => "application/manifest+json",
        "wasm" => "application/wasm",
        "pdf" => "application/pdf",
        "mp3" => "audio/mpeg",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        _ => "application/octet-stream",
    }
}

/// `path.Clean("/" + p)` without the leading slash; empty for the root.
fn clean(p: &str) -> String {
    let parts: Vec<&str> = p.split('/').filter(|s| !s.is_empty() && *s != ".").collect();
    parts.join("/")
}

/// The decoded request path; `None` when an escape is malformed or the
/// bytes are not UTF-8.
fn percent_decode(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let hex = s.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn nosniff(mut r: Response) -> Response {
    r.headers_mut().insert("x-content-type-options", HeaderValue::from_static("nosniff"));
    r
}

fn json_error(status: StatusCode, msg: &str) -> Response {
    let mut r = Response::new(Body::from(format!("{{\"error\":\"{msg}\"}}\n")));
    *r.status_mut() = status;
    r.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("application/json"));
    r
}

/// One embedded file, with HEAD and a single byte range like Go's
/// `http.ServeContent` (embedded files have no modification time, so no
/// `Last-Modified` and no conditional answers; several ranges get the
/// whole file).
fn content(
    method: &Method,
    req: &HeaderMap,
    ctype: &'static str,
    cache: &'static str,
    bytes: &'static [u8],
) -> Response {
    let size = bytes.len() as u64;
    let mut h = HeaderMap::new();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static(ctype));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    h.insert("x-content-type-options", HeaderValue::from_static("nosniff"));
    h.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    let range = req.get(header::RANGE).and_then(|v| v.to_str().ok()).unwrap_or("");
    let (status, body): (StatusCode, &'static [u8]) = match parse_range(range, size) {
        Ok(r) if r.len() == 1 => {
            let (s, n) = r[0];
            if let Ok(v) = HeaderValue::from_str(&format!("bytes {s}-{}/{size}", s + n - 1)) {
                h.insert(header::CONTENT_RANGE, v);
            }
            (StatusCode::PARTIAL_CONTENT, &bytes[s as usize..(s + n) as usize])
        }
        Ok(_) => (StatusCode::OK, bytes),
        Err(e) => {
            if e == RangeError::NoOverlap
                && let Ok(v) = HeaderValue::from_str(&format!("bytes */{size}"))
            {
                h.insert(header::CONTENT_RANGE, v);
            }
            h.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/plain; charset=utf-8"));
            let text: &'static [u8] =
                if e == RangeError::NoOverlap { b"invalid range: failed to overlap\n" } else { b"invalid range\n" };
            (StatusCode::RANGE_NOT_SATISFIABLE, text)
        }
    };
    if let Ok(v) = HeaderValue::from_str(&body.len().to_string()) {
        h.insert(header::CONTENT_LENGTH, v);
    }
    let mut r = if *method == Method::HEAD { Response::new(Body::empty()) } else { Response::new(Body::from(body)) };
    *r.status_mut() = status;
    *r.headers_mut() = h;
    r
}

/// What a binary built without the web UI serves: plain, honest and
/// useful for development.
const NOT_BUILT: &str = r#"<!doctype html>
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
<p>Build it with <code>just web</code> and rebuild <code>lain</code> (needs pnpm once), or
develop against the Vite dev server: <code>cd web &amp;&amp; pnpm dev</code>.</p>
</main>
</body>
</html>
"#;

#[cfg(test)]
mod tests {
    use super::*;

    static TREE: &[(&str, &[u8])] = &[
        ("index.html", b"<!doctype html><title>lain</title>"),
        ("_app/immutable/entry/start.abc123.js", b"export const x = 1;"),
        ("favicon.svg", b"<svg/>"),
        ("robots.txt", b"User-agent: *\n"),
        ("docs/index.html", b"<p>nested</p>"),
    ];

    fn get(w: WebAssets, method: Method, path: &str, range: Option<&str>) -> (StatusCode, HeaderMap, Vec<u8>) {
        let mut req = HeaderMap::new();
        if let Some(r) = range {
            req.insert(header::RANGE, HeaderValue::from_str(r).unwrap());
        }
        let r = w.respond(&method, path, &req);
        let (parts, body) = r.into_parts();
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        let bytes = rt.block_on(axum::body::to_bytes(body, 1 << 20)).unwrap().to_vec();
        (parts.status, parts.headers, bytes)
    }

    fn hdr<'a>(h: &'a HeaderMap, k: &str) -> &'a str {
        h.get(k).and_then(|v| v.to_str().ok()).unwrap_or("")
    }

    #[test]
    fn spa_routes_serve_the_index_uncached() {
        let w = WebAssets::new(TREE);
        for p in ["/", "", "/library", "/items/abc/play", "/settings/server/", "/docs"] {
            let (s, h, b) = get(w, Method::GET, p, None);
            assert_eq!(s, StatusCode::OK, "{p}");
            assert_eq!(hdr(&h, "cache-control"), "no-cache", "{p}");
            assert_eq!(hdr(&h, "content-type"), "text/html; charset=utf-8");
            assert_eq!(b, TREE[0].1, "{p}");
        }
    }

    #[test]
    fn assets_carry_types_and_caching() {
        let w = WebAssets::new(TREE);
        let (s, h, b) = get(w, Method::GET, "/_app/immutable/entry/start.abc123.js", None);
        assert_eq!((s, b.as_slice()), (StatusCode::OK, &b"export const x = 1;"[..]));
        assert_eq!(hdr(&h, "content-type"), "text/javascript; charset=utf-8");
        assert_eq!(hdr(&h, "cache-control"), "public, max-age=31536000, immutable");
        assert_eq!(hdr(&h, "x-content-type-options"), "nosniff");
        let (_, h, _) = get(w, Method::GET, "/favicon.svg", None);
        assert_eq!(hdr(&h, "content-type"), "image/svg+xml");
        assert_eq!(hdr(&h, "cache-control"), "public, max-age=3600");
        // A nested index is a file, not a directory listing.
        let (s, _, b) = get(w, Method::GET, "/docs/index.html", None);
        assert_eq!((s, b.as_slice()), (StatusCode::OK, &b"<p>nested</p>"[..]));
    }

    #[test]
    fn missing_assets_are_404_and_api_never_html() {
        let w = WebAssets::new(TREE);
        for p in ["/_app/immutable/gone.js", "/missing.css", "/x/y.png", "/font.woff2", "/_app/version"] {
            let (s, h, b) = get(w, Method::GET, p, None);
            assert_eq!(s, StatusCode::NOT_FOUND, "{p}");
            assert_eq!(hdr(&h, "content-type"), "application/json");
            assert_eq!(b, b"{\"error\":\"not found\"}\n");
        }
        for p in ["/api", "/api/", "/api/nope", "/health", "/health/deep"] {
            let (s, h, _) = get(w, Method::GET, p, None);
            assert_eq!(s, StatusCode::NOT_FOUND, "{p}");
            assert_eq!(hdr(&h, "content-type"), "application/json");
        }
    }

    #[test]
    fn traversal_and_methods_are_refused() {
        let w = WebAssets::new(TREE);
        for p in ["/../etc/passwd", "/a/%2e%2e/b", "/x%00y", "/%zz"] {
            let (s, _, b) = get(w, Method::GET, p, None);
            assert_eq!(s, StatusCode::BAD_REQUEST, "{p}");
            assert_eq!(b, b"{\"error\":\"bad path\"}\n");
        }
        let (s, h, _) = get(w, Method::POST, "/library", None);
        assert_eq!(s, StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(hdr(&h, "allow"), "GET, HEAD");
    }

    #[test]
    fn head_and_ranges() {
        let w = WebAssets::new(TREE);
        let (s, h, b) = get(w, Method::HEAD, "/robots.txt", None);
        assert_eq!(s, StatusCode::OK);
        assert_eq!(hdr(&h, "content-length"), "14");
        assert!(b.is_empty());
        let (s, h, b) = get(w, Method::GET, "/robots.txt", Some("bytes=0-3"));
        assert_eq!((s, b.as_slice()), (StatusCode::PARTIAL_CONTENT, &b"User"[..]));
        assert_eq!(hdr(&h, "content-range"), "bytes 0-3/14");
        let (s, h, _) = get(w, Method::GET, "/robots.txt", Some("bytes=99-"));
        assert_eq!(s, StatusCode::RANGE_NOT_SATISFIABLE);
        assert_eq!(hdr(&h, "content-range"), "bytes */14");
    }

    #[test]
    fn a_build_without_the_web_ui_says_so() {
        let w = WebAssets::none();
        assert!(!w.is_built());
        let (s, h, b) = get(w, Method::GET, "/library", None);
        assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(hdr(&h, "content-type"), "text/html; charset=utf-8");
        assert!(String::from_utf8(b).unwrap().contains("built without the web UI"));
        // Asset paths still 404 as JSON.
        assert_eq!(get(w, Method::GET, "/app.js", None).0, StatusCode::NOT_FOUND);
    }
}
