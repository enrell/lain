package gateway

// mutation-clean: gremlins v0.6.0 — package verified 2026-09-22

import (
	"encoding/json"
	"strings"
	"testing"
	"time"

	"github.com/enrell/lain/internal/contracts"
)

// TestTranscodeHLSServesPlaylistAndSegments is the real HLS slice: a
// synthesized MKV starts an HLS session, the server-owned playlist
// becomes playable while ffmpeg still runs (or as soon as it finishes
// on short sources), and the init segment plus media segments are
// served from the session directory without leaking paths.
func TestTranscodeHLSServesPlaylistAndSegments(t *testing.T) {
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	id := catalogOneMKV(t, srv, admin, t.TempDir(), "Show.mkv")

	rec := do(t, srv, "POST", "/api/items/"+id+"/transcode", map[string]any{"delivery": "hls"}, admin)
	if rec.Code != 200 && rec.Code != 202 {
		t.Fatalf("start: %d %s", rec.Code, rec.Body.String())
	}
	var status struct {
		Session  string `json:"session"`
		State    string `json:"state"`
		Delivery string `json:"delivery"`
		Playable bool   `json:"playable"`
	}
	if err := json.Unmarshal(rec.Body.Bytes(), &status); err != nil || status.Session == "" {
		t.Fatalf("start status: %d %s", rec.Code, rec.Body.String())
	}
	if status.Delivery != contracts.TranscodeDeliveryHLS {
		t.Fatalf("delivery=%q, want hls", status.Delivery)
	}

	// Wait for a ready session; short sources finish quickly.
	deadline := time.Now().Add(20 * time.Second)
	for status.State != contracts.TranscodeReady && time.Now().Before(deadline) {
		time.Sleep(20 * time.Millisecond)
		rec = do(t, srv, "GET", "/api/items/"+id+"/transcode/status?session="+status.Session, nil, admin)
		if err := json.Unmarshal(rec.Body.Bytes(), &status); err != nil {
			t.Fatal(err)
		}
	}
	if status.State != contracts.TranscodeReady || !status.Playable {
		t.Fatalf("status=%+v, want ready+playable", status)
	}
	if strings.Contains(rec.Body.String(), "transcodes/") || strings.Contains(rec.Body.String(), ".m3u8") {
		t.Fatalf("status leaks trusted paths: %s", rec.Body.String())
	}

	base := "/api/items/" + id + "/transcode/hls/"
	q := "?session=" + status.Session + "&token=" + admin

	rec = do(t, srv, "GET", base+"index.m3u8"+q, nil, "")
	if rec.Code != 200 {
		t.Fatalf("playlist: %d %s", rec.Code, rec.Body.String())
	}
	if ct := rec.Header().Get("Content-Type"); ct != "application/vnd.apple.mpegurl" {
		t.Fatalf("playlist content-type %q", ct)
	}
	playlist := rec.Body.String()
	if !strings.HasPrefix(playlist, "#EXTM3U") {
		t.Fatalf("bad playlist:\n%s", playlist)
	}
	if !strings.Contains(playlist, "#EXT-X-ENDLIST") {
		t.Fatalf("finished session playlist lacks ENDLIST:\n%s", playlist)
	}

	// The playlist must sign its media URIs. A relative URI inside an m3u8
	// drops the playlist's query string, so the browser would fetch the init
	// segment and the media segments unauthenticated and get 401 — HLS never
	// played. This is the regression guard for that bug.
	initURI := playlistMapURI(t, playlist)
	if !strings.Contains(initURI, "session="+status.Session) || !strings.Contains(initURI, "token="+admin) {
		t.Fatalf("EXT-X-MAP URI not signed: %q", initURI)
	}
	segURI := playlistSegmentURI(t, playlist)
	if !strings.Contains(segURI, "session="+status.Session) || !strings.Contains(segURI, "token="+admin) {
		t.Fatalf("segment URI not signed: %q", segURI)
	}

	// Fetching exactly the URIs the playlist advertises, with no extra
	// credentials, must authenticate: this is what hls.js and Safari do.
	rec = do(t, srv, "GET", base+initURI, nil, "")
	if rec.Code != 200 || rec.Body.Len() == 0 {
		t.Fatalf("init segment: %d %d bytes", rec.Code, rec.Body.Len())
	}
	body := rec.Body.Bytes()
	if len(body) < 8 || string(body[4:8]) != "ftyp" {
		t.Fatalf("init segment is not fMP4: % x", body[:min(8, len(body))])
	}

	rec = do(t, srv, "GET", base+segURI, nil, "")
	if rec.Code != 200 || rec.Body.Len() == 0 {
		t.Fatalf("media segment: %d %d bytes", rec.Code, rec.Body.Len())
	}

	// Unknown files inside the session directory are not reachable.
	if rec := do(t, srv, "GET", base+"raw.m3u8"+q, nil, ""); rec.Code != 404 {
		t.Fatalf("raw playlist reachable: %d", rec.Code)
	}
	// A traversal attempt must never serve a file; ServeMux may
	// canonicalize the path (3xx) before the handler rejects it.
	if rec := do(t, srv, "GET", base+"..%2F..%2Flain.db"+q, nil, ""); rec.Code == 200 {
		t.Fatalf("traversal served: %d", rec.Code)
	}
}

// playlistMapURI returns the EXT-X-MAP URI attribute value.
func playlistMapURI(t *testing.T, playlist string) string {
	t.Helper()
	const attr = `#EXT-X-MAP:URI="`
	i := strings.Index(playlist, attr)
	if i < 0 {
		t.Fatalf("playlist has no EXT-X-MAP:\n%s", playlist)
	}
	rest := playlist[i+len(attr):]
	j := strings.Index(rest, `"`)
	if j < 0 {
		t.Fatalf("EXT-X-MAP URI is unterminated:\n%s", playlist)
	}
	return rest[:j]
}

// playlistSegmentURI returns the first non-tag media URI line.
func playlistSegmentURI(t *testing.T, playlist string) string {
	t.Helper()
	for _, line := range strings.Split(playlist, "\n") {
		trimmed := strings.TrimSpace(line)
		if trimmed == "" || strings.HasPrefix(trimmed, "#") {
			continue
		}
		return trimmed
	}
	t.Fatalf("playlist has no segment URI:\n%s", playlist)
	return ""
}

// TestRewriteHLSPlaylistSignsMediaURIs locks the pure rewrite: tags and
// blank lines stay byte-identical and only the EXT-X-MAP URI plus the
// segment lines gain the session and token.
func TestRewriteHLSPlaylistSignsMediaURIs(t *testing.T) {
	in := "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:6\n#EXT-X-MAP:URI=\"init.mp4\"\n#EXTINF:6.000,\nseg00000.m4s\n#EXT-X-ENDLIST\n"
	out := rewriteHLSPlaylist(in, "sess123", "tok.abc")
	for _, want := range []string{
		"#EXTM3U\n",
		"#EXT-X-VERSION:7\n",
		"#EXT-X-TARGETDURATION:6\n",
		"#EXT-X-ENDLIST\n",
		`#EXT-X-MAP:URI="init.mp4?session=sess123&token=tok.abc"`,
		"seg00000.m4s?session=sess123&token=tok.abc",
	} {
		if !strings.Contains(out, want) {
			t.Fatalf("rewritten playlist missing %q:\n%s", want, out)
		}
	}
	if strings.Contains(out, `URI="init.mp4"`) {
		t.Fatalf("EXT-X-MAP URI left unsigned:\n%s", out)
	}
}

// TestHLSFileWhitelist pins the servable HLS file names for both
// containers: fMP4 (.m4s) and mpegts (.ts) segments, the playlist and
// the init segment, and nothing else (no traversal, no stray files).
func TestHLSFileWhitelist(t *testing.T) {
	allowed := []string{"index.m3u8", "init.mp4", "seg00000.m4s", "seg00001.m4s", "seg00000.ts", "seg00042.ts"}
	for _, name := range allowed {
		if !validHLSFile(name) {
			t.Errorf("validHLSFile(%q) = false, want true", name)
		}
	}
	denied := []string{"../raw.m3u8", "raw.m3u8", "seg.ts", "seg-1.ts", "seg00000.mp4", "index.m3u8/../x", "seg00000.ts.tmp"}
	for _, name := range denied {
		if validHLSFile(name) {
			t.Errorf("validHLSFile(%q) = true, want false", name)
		}
	}
}
