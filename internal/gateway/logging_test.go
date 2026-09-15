package gateway

import (
	"bytes"
	"context"
	"encoding/json"
	"log/slog"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/enrell/lain/internal/contracts"
)

// bufLogger installs the production-shaped agent logger over a buffer
// at the given level and returns the buffer for assertions.
func bufLogger(t *testing.T, srv *Server, level slog.Level) *bytes.Buffer {
	t.Helper()
	var buf bytes.Buffer
	srv.SetLogger(NewAgentLogger(&buf, level, "lain", "test"))
	return &buf
}

func TestParseLogLevel(t *testing.T) {
	for _, tc := range []struct {
		in   string
		want slog.Level
	}{
		{"debug", slog.LevelDebug},
		{"DEBUG", slog.LevelDebug},
		{" info ", slog.LevelInfo},
		{"", slog.LevelInfo},
		{"warn", slog.LevelWarn},
		{"warning", slog.LevelWarn},
		{"error", slog.LevelError},
	} {
		got, err := ParseLogLevel(tc.in)
		if err != nil || got != tc.want {
			t.Fatalf("ParseLogLevel(%q)=%v,%v want %v", tc.in, got, err, tc.want)
		}
	}
	if _, err := ParseLogLevel("verbose"); err == nil {
		t.Fatal("unknown level must fail")
	}
}

func TestAgentLoggerEnvelope(t *testing.T) {
	srv := testServer(t)
	buf := bufLogger(t, srv, slog.LevelDebug)

	if rec := do(t, srv, "GET", "/api/items/nope/thumbnail", nil, ""); rec.Code != 401 {
		t.Fatalf("route: %d", rec.Code)
	}
	out := buf.String()
	// OpenTelemetry-shaped envelope: Timestamp, SeverityText,
	// SeverityNumber, Body, Resource, flat Attributes, req correlation.
	for _, want := range []string{
		`"Timestamp":"20`,
		`"SeverityText":"WARN"`,
		`"SeverityNumber":13`,
		`"Body":"request"`,
		`"Resource":{"service.name":"lain","service.version":"test"}`,
		`"req":"req-1"`,
		`"path":"/api/items/nope/thumbnail"`,
		`"status":401`,
	} {
		if !strings.Contains(out, want) {
			t.Fatalf("envelope missing %s:\n%s", want, out)
		}
	}
	// SeverityNumber bands follow the OTel spec ranges.
	for _, tc := range []struct {
		level slog.Level
		want  string
	}{
		{slog.LevelDebug, `"SeverityNumber":5`},
		{slog.LevelInfo, `"SeverityNumber":9`},
		{slog.LevelError, `"SeverityNumber":17`},
	} {
		var b bytes.Buffer
		NewAgentLogger(&b, slog.LevelDebug, "lain", "test").Log(context.Background(), tc.level, "probe")
		if !strings.Contains(b.String(), tc.want) {
			t.Fatalf("level %v missing %s:\n%s", tc.level, tc.want, b.String())
		}
	}
}

func TestAccessLogLine(t *testing.T) {
	srv := testServer(t)
	buf := bufLogger(t, srv, slog.LevelDebug)

	rec := do(t, srv, "GET", "/health", nil, "")
	if rec.Code != 200 {
		t.Fatalf("health: %d", rec.Code)
	}
	rec = do(t, srv, "GET", "/health", nil, "")
	if rec.Code != 200 {
		t.Fatalf("health: %d", rec.Code)
	}
	out := buf.String()
	for _, want := range []string{`"Body":"request"`, `"method":"GET"`, `"path":"/health"`, `"status":200`, `"req":"req-1"`, `"req":"req-2"`} {
		if !strings.Contains(out, want) {
			t.Fatalf("access line missing %s:\n%s", want, out)
		}
	}
}

// Query strings carry media tokens and bodies carry passwords: access
// lines log path only, never either.
func TestAccessLogHidesSecrets(t *testing.T) {
	srv := testServer(t)
	buf := bufLogger(t, srv, slog.LevelDebug)

	rec := do(t, srv, "POST", "/api/auth/login?token=tok-secret", map[string]string{"username": "nobody", "password": "password123"}, "")
	if rec.Code != 401 {
		t.Fatalf("login: %d %s", rec.Code, rec.Body.String())
	}
	out := buf.String()
	for _, secret := range []string{"tok-secret", "password123"} {
		if strings.Contains(out, secret) {
			t.Fatalf("secret %q leaked into logs:\n%s", secret, out)
		}
	}
	if !strings.Contains(out, `"path":"/api/auth/login"`) {
		t.Fatalf("path missing from access line:\n%s", out)
	}
}

func enrichFixture(t *testing.T, srv *Server, admin, name string) string {
	t.Helper()
	root := t.TempDir()
	if err := os.WriteFile(filepath.Join(root, name), []byte("x"), 0o644); err != nil {
		t.Fatal(err)
	}
	if rec := do(t, srv, "POST", "/api/libraries", map[string]string{"name": "L", "type": "anime", "path": root}, admin); rec.Code != 201 {
		t.Fatalf("library: %d %s", rec.Code, rec.Body.String())
	}
	if rec := do(t, srv, "POST", "/api/library/scan", nil, admin); rec.Code != 202 {
		t.Fatalf("scan: %d %s", rec.Code, rec.Body.String())
	}
	var status struct {
		State string `json:"state"`
	}
	for i := 0; i < 100; i++ {
		rec := do(t, srv, "GET", "/api/library/scan", nil, admin)
		_ = json.Unmarshal(rec.Body.Bytes(), &status)
		if status.State == "done" {
			break
		}
	}
	var page struct {
		Items []map[string]any `json:"items"`
	}
	rec := do(t, srv, "GET", "/api/catalog", nil, admin)
	_ = json.Unmarshal(rec.Body.Bytes(), &page)
	return page.Items[0]["id"].(string)
}

// A dead upstream used to vanish inside the merge; now the search
// failure and its cause are one grep away.
func TestEnrichLogsSearchFailure(t *testing.T) {
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	buf := bufLogger(t, srv, slog.LevelDebug)
	withFakeMetadata(t, srv,
		&fakeMeta{id: "lain-metadata-nfo", failSearch: true},
		&fakeMeta{id: "lain-metadata-kitsu", failSearch: true},
		&fakeMeta{id: "lain-metadata-anilist", failSearch: true},
		&fakeMeta{id: "lain-metadata-jikan", failSearch: true},
		&fakeMeta{id: "lain-metadata-tvmaze", failSearch: true},
	)
	id := enrichFixture(t, srv, admin, "Whatever.mkv")
	if rec := do(t, srv, "POST", "/api/catalog/"+id+"/enrich", nil, admin); rec.Code != 503 {
		t.Fatalf("all down must 503, got %d %s", rec.Code, rec.Body.String())
	}
	out := buf.String()
	for _, want := range []string{`"Body":"enrich search failed"`, `"item":"` + id + `"`, "lain-metadata-tvmaze: upstream down"} {
		if !strings.Contains(out, want) {
			t.Fatalf("attribution missing %s:\n%s", want, out)
		}
	}
}

// An empty merge still 404s with the identical body; the cause lives
// in the warn line, not the wire.
func TestEnrichLogsNoMatch(t *testing.T) {
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	buf := bufLogger(t, srv, slog.LevelDebug)
	withFakeMetadata(t, srv,
		&fakeMeta{id: "lain-metadata-nfo"},
		&fakeMeta{id: "lain-metadata-kitsu"},
		&fakeMeta{id: "lain-metadata-anilist"},
		&fakeMeta{id: "lain-metadata-jikan"},
		&fakeMeta{id: "lain-metadata-tvmaze"},
	)
	id := enrichFixture(t, srv, admin, "Whatever.mkv")
	rec := do(t, srv, "POST", "/api/catalog/"+id+"/enrich", nil, admin)
	if rec.Code != 404 {
		t.Fatalf("empty merge must 404, got %d %s", rec.Code, rec.Body.String())
	}
	if body := rec.Body.String(); !strings.Contains(body, "no metadata found") {
		t.Fatalf("body changed: %s", body)
	}
	out := buf.String()
	for _, want := range []string{`"Body":"enrich no match"`, `"item":"` + id + `"`, `"title":"Whatever"`} {
		if !strings.Contains(out, want) {
			t.Fatalf("attribution missing %s:\n%s", want, out)
		}
	}
}

func TestEnrichLogsWinner(t *testing.T) {
	srv := testServer(t)
	admin := setupAdmin(t, srv)
	buf := bufLogger(t, srv, slog.LevelDebug)
	withFakeMetadata(t, srv,
		&fakeMeta{id: "lain-metadata-nfo", candidates: []contracts.MetadataCandidate{}},
		&fakeMeta{
			id:         "lain-metadata-kitsu",
			candidates: []contracts.MetadataCandidate{{Provider: "lain-metadata-kitsu", RemoteID: "12", Title: "Frieren", Year: 2023}},
			record:     contracts.MetadataRecord{Provider: "lain-metadata-kitsu", RemoteID: "12", Title: "Frieren", Year: 2023},
		},
		&fakeMeta{id: "lain-metadata-anilist", failSearch: true},
		&fakeMeta{id: "lain-metadata-jikan", failSearch: true},
		&fakeMeta{id: "lain-metadata-tvmaze", failSearch: true},
	)
	id := enrichFixture(t, srv, admin, "Frieren - 12.mkv")
	if rec := do(t, srv, "POST", "/api/catalog/"+id+"/enrich", nil, admin); rec.Code != 200 {
		t.Fatalf("enrich: %d %s", rec.Code, rec.Body.String())
	}
	out := buf.String()
	for _, want := range []string{`"Body":"enrich ok"`, `"provider":"lain-metadata-kitsu"`, "lain-metadata-tvmaze: upstream down"} {
		if !strings.Contains(out, want) {
			t.Fatalf("diagnostics missing %s:\n%s", want, out)
		}
	}
}
