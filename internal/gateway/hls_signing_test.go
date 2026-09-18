package gateway

// Adversarial audit of the newest HLS signing surface: the playlist the
// gateway serves must sign every media URI with the session and the
// caller's token, and those signed URIs are what authorizes init.mp4 and
// the segments — hls.js and Safari cannot attach an Authorization header.

import (
	"encoding/json"
	"net/http"
	"net/url"
	"strings"
	"testing"
	"time"

	"github.com/enrell/lain/internal/contracts"
)

// TestRewriteHLSPlaylistSignsTagsAndMedia locks the byte-exact rewrite:
// every tag survives untouched, both segment lines and the EXT-X-MAP URI
// gain session+token, and the trailing newline stays.
func TestRewriteHLSPlaylistSignsTagsAndMedia(t *testing.T) {
	in := "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:6\n" +
		`#EXT-X-MAP:URI="init.mp4"` + "\n" +
		"#EXTINF:6.000,\nseg00000.m4s\n" +
		"#EXTINF:6.000,\nseg00001.m4s\n" +
		"#EXT-X-ENDLIST\n"
	want := "#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-TARGETDURATION:6\n" +
		`#EXT-X-MAP:URI="init.mp4?session=sess-1&token=tok-1"` + "\n" +
		"#EXTINF:6.000,\nseg00000.m4s?session=sess-1&token=tok-1\n" +
		"#EXTINF:6.000,\nseg00001.m4s?session=sess-1&token=tok-1\n" +
		"#EXT-X-ENDLIST\n"
	out := rewriteHLSPlaylist(in, "sess-1", "tok-1")
	if out != want {
		t.Fatalf("rewriteHLSPlaylist mismatch:\n got %q\nwant %q", out, want)
	}
}

// TestRewriteHLSPlaylistLeavesNonMediaTagsAlone pins the map-line guard:
// an EXT-X-MAP without a URI attribute (or with an unterminated one) is
// not a media URI and must come back byte-identical.
func TestRewriteHLSPlaylistLeavesNonMediaTagsAlone(t *testing.T) {
	for _, line := range []string{
		`#EXT-X-MAP:BYTERANGE="1024@0"`,
		`#EXT-X-MAP:URI="init.mp4`,
	} {
		in := "#EXTM3U\n" + line + "\nseg00000.m4s\n"
		out := rewriteHLSPlaylist(in, "sess-2", "tok-2")
		if !strings.Contains(out, line+"\n") {
			t.Errorf("map line %q was rewritten:\n%s", line, out)
		}
		if !strings.Contains(out, "seg00000.m4s?session=sess-2&token=tok-2") {
			t.Errorf("segment not signed next to %q:\n%s", line, out)
		}
	}
}

// TestRewriteHLSPlaylistEmptyToken pins the token absence: the session
// is mandatory for the handler, the token is only appended when one was
// presented.
func TestRewriteHLSPlaylistEmptyToken(t *testing.T) {
	out := rewriteHLSPlaylist("#EXTM3U\n"+`#EXT-X-MAP:URI="init.mp4"`+"\nseg00000.m4s\n", "sess-3", "")
	if !strings.Contains(out, `URI="init.mp4?session=sess-3"`) {
		t.Fatalf("EXT-X-MAP URI without a token was not signed with the session:\n%s", out)
	}
	if !strings.Contains(out, "seg00000.m4s?session=sess-3") {
		t.Fatalf("segment without a token was not signed with the session:\n%s", out)
	}
	if strings.Contains(out, "token=") {
		t.Fatalf("empty token was still appended:\n%s", out)
	}
}

// TestRewriteHLSPlaylistKeepsExistingQuery pins query preservation: a
// media URI that already carries parameters keeps them alongside the
// signing pair instead of losing them to the rewrite.
func TestRewriteHLSPlaylistKeepsExistingQuery(t *testing.T) {
	out := rewriteHLSPlaylist("#EXTM3U\nseg00000.m4s?v=2&name=a%20b\n", "sess-4", "tok-4")
	signed := playlistSegmentURI(t, out)
	u, err := url.Parse(signed)
	if err != nil {
		t.Fatalf("signed URI %q: %v", signed, err)
	}
	if u.Path != "seg00000.m4s" {
		t.Fatalf("signed URI path=%q, want seg00000.m4s", u.Path)
	}
	q := u.Query()
	for key, want := range map[string]string{
		"v": "2", "name": "a b", "session": "sess-4", "token": "tok-4",
	} {
		if got := q.Get(key); got != want {
			t.Errorf("query %s=%q, want %q (in %q)", key, got, want, signed)
		}
	}
}

// TestHLSSignedURIsAuthenticateExactlyAsAdvertised is the browser
// contract: the playlist fetched with ?session&token advertises signed
// URIs, fetching those exact URIs with no extra credentials serves
// init.mp4 and the first segment, and removing only the token must 401 —
// proving the signature, not an open endpoint, authorizes playback.
func TestHLSSignedURIsAuthenticateExactlyAsAdvertised(t *testing.T) {
	e2eRequireFFmpeg(t)
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	id := catalogOneMKV(t, srv, admin, t.TempDir(), "[Fansub-A] Signing Show.mkv")
	session := signStartHLS(t, srv, admin, id)

	base := "/api/items/" + id + "/transcode/hls/"
	rec := do(t, srv, "GET", base+"index.m3u8?session="+session+"&token="+admin, nil, "")
	if rec.Code != http.StatusOK {
		t.Fatalf("playlist: %d %s", rec.Code, rec.Body.String())
	}
	playlist := rec.Body.String()
	initURI := playlistMapURI(t, playlist)
	segURI := playlistSegmentURI(t, playlist)
	if !strings.Contains(initURI, "session="+session) || !strings.Contains(initURI, "token="+admin) {
		t.Fatalf("EXT-X-MAP URI not signed for this session: %q", initURI)
	}
	if !strings.Contains(segURI, "session="+session) || !strings.Contains(segURI, "token="+admin) {
		t.Fatalf("segment URI not signed for this session: %q", segURI)
	}

	// Fetch exactly what the playlist advertised, without credentials:
	// this is what hls.js and Safari do.
	rec = do(t, srv, "GET", base+initURI, nil, "")
	if rec.Code != http.StatusOK || rec.Body.Len() == 0 {
		t.Fatalf("init segment as advertised: %d (%d bytes)", rec.Code, rec.Body.Len())
	}
	if body := rec.Body.Bytes(); len(body) < 8 || string(body[4:8]) != "ftyp" {
		t.Fatalf("init segment is not fMP4: % x", body[:min(8, len(body))])
	}
	rec = do(t, srv, "GET", base+segURI, nil, "")
	if rec.Code != http.StatusOK || rec.Body.Len() == 0 {
		t.Fatalf("first segment as advertised: %d (%d bytes)", rec.Code, rec.Body.Len())
	}

	// The same URIs with only the token removed: the session alone must
	// not authorize media.
	for _, uri := range []string{initURI, segURI} {
		stripped := signStripToken(t, uri)
		if rec := do(t, srv, "GET", base+stripped, nil, ""); rec.Code != http.StatusUnauthorized {
			t.Fatalf("unsigned media %q returned %d, want 401", stripped, rec.Code)
		}
	}
}

// TestHLSPlaylistSignsBearerHeaderToken pins requestToken's header
// fallback: a playlist fetched with Authorization: Bearer must embed
// that token into the signed URIs, so the header-less media fetches
// still authenticate.
func TestHLSPlaylistSignsBearerHeaderToken(t *testing.T) {
	e2eRequireFFmpeg(t)
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	id := catalogOneMKV(t, srv, admin, t.TempDir(), "[Fansub-A] Bearer Signing.mkv")
	session := signStartHLS(t, srv, admin, id)

	base := "/api/items/" + id + "/transcode/hls/"
	rec := do(t, srv, "GET", base+"index.m3u8?session="+session, nil, admin)
	if rec.Code != http.StatusOK {
		t.Fatalf("playlist via bearer header: %d %s", rec.Code, rec.Body.String())
	}
	playlist := rec.Body.String()
	initURI := playlistMapURI(t, playlist)
	segURI := playlistSegmentURI(t, playlist)
	if !strings.Contains(initURI, "token="+admin) || !strings.Contains(segURI, "token="+admin) {
		t.Fatalf("bearer token missing from signed URIs: map=%q seg=%q", initURI, segURI)
	}
	for _, uri := range []string{initURI, segURI} {
		rec := do(t, srv, "GET", base+uri, nil, "")
		if rec.Code != http.StatusOK || rec.Body.Len() == 0 {
			t.Fatalf("advertised %q from a bearer-signed playlist: %d (%d bytes)", uri, rec.Code, rec.Body.Len())
		}
	}
}

// signStartHLS starts an HLS session for one item and waits for the
// plugin to report ready+playable; preparation is asynchronous, so a
// deadline is the only safe wait.
func signStartHLS(t *testing.T, srv *Server, admin, id string) string {
	t.Helper()
	rec := do(t, srv, "POST", "/api/items/"+id+"/transcode", map[string]any{"delivery": "hls"}, admin)
	if rec.Code != http.StatusOK && rec.Code != http.StatusAccepted {
		t.Fatalf("start: %d %s", rec.Code, rec.Body.String())
	}
	var status struct {
		Session  string `json:"session"`
		State    string `json:"state"`
		Playable bool   `json:"playable"`
	}
	if err := json.Unmarshal(rec.Body.Bytes(), &status); err != nil || status.Session == "" {
		t.Fatalf("start status: %d %s", rec.Code, rec.Body.String())
	}
	deadline := time.Now().Add(30 * time.Second)
	for status.State != contracts.TranscodeReady && time.Now().Before(deadline) {
		time.Sleep(20 * time.Millisecond)
		rec = do(t, srv, "GET", "/api/items/"+id+"/transcode/status?session="+status.Session, nil, admin)
		if err := json.Unmarshal(rec.Body.Bytes(), &status); err != nil {
			t.Fatalf("status: %v (%s)", err, rec.Body.String())
		}
		if status.State == contracts.TranscodeFailed {
			t.Fatalf("session failed: %s", rec.Body.String())
		}
	}
	if status.State != contracts.TranscodeReady || !status.Playable {
		t.Fatalf("session %+v, want ready+playable", status)
	}
	return status.Session
}

// signStripToken removes only the token parameter, keeping the session,
// so a 401 proves the token — not an open endpoint — authorizes media.
func signStripToken(t *testing.T, uri string) string {
	t.Helper()
	u, err := url.Parse(uri)
	if err != nil {
		t.Fatalf("parse advertised URI %q: %v", uri, err)
	}
	q := u.Query()
	if q.Get("token") == "" {
		t.Fatalf("advertised URI %q carries no token", uri)
	}
	q.Del("token")
	u.RawQuery = q.Encode()
	return u.String()
}
