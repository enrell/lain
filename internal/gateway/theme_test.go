package gateway

import (
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
)

func TestThemeEndpointPublishesNormalizedOmarchyPaletteWithoutAuth(t *testing.T) {
	srv := testServer(t)
	dir := t.TempDir()
	path := filepath.Join(dir, "colors.toml")
	colors := `mode = "light"
background = "#FFFCF0"
dark_background = "#F2EFE4"
lighter_background = "#E6E4D9"
foreground = "#100F0F"
muted = "#878580"
selection = "#CECDC3"
accent = "#205EA6"
bright_blue = "#4385BE"
red = "#D14D41"
green = "#879A39"
yellow = "#D0A215"
`
	if err := os.WriteFile(path, []byte(colors), 0o644); err != nil {
		t.Fatal(err)
	}
	srv.themePath = path

	rec := do(t, srv, "GET", "/api/theme", nil, "")
	if rec.Code != 200 {
		t.Fatalf("theme: %d %s", rec.Code, rec.Body.String())
	}
	if got := rec.Header().Get("Cache-Control"); got != "no-store" {
		t.Fatalf("Cache-Control = %q, want no-store", got)
	}
	var got themePalette
	if err := json.Unmarshal(rec.Body.Bytes(), &got); err != nil {
		t.Fatal(err)
	}
	if got.Source != "omarchy" || got.Mode != "light" {
		t.Fatalf("source/mode = %q/%q", got.Source, got.Mode)
	}
	if got.Background != "#fffcf0" || got.Surface != "#f2efe4" || got.SurfaceHover != "#e6e4d9" {
		t.Fatalf("background surfaces not normalized: %+v", got)
	}
	if got.Accent != "#205ea6" || got.AccentHover != "#4385be" || got.AccentForeground != "#ffffff" {
		t.Fatalf("accent mapping wrong: %+v", got)
	}
	if got.Danger != "#d14d41" || got.DangerForeground != "#000000" || got.Success != "#879a39" || got.Warning != "#d0a215" {
		t.Fatalf("status mapping wrong: %+v", got)
	}
}

func TestThemeEndpointFallsBackOnMissingOrUnsafePalette(t *testing.T) {
	srv := testServer(t)
	srv.themePath = filepath.Join(t.TempDir(), "missing.toml")

	rec := do(t, srv, "GET", "/api/theme", nil, "")
	var got themePalette
	if err := json.Unmarshal(rec.Body.Bytes(), &got); err != nil {
		t.Fatal(err)
	}
	if rec.Code != 200 || got.Source != "default" || got.Background != "#08090c" {
		t.Fatalf("missing theme fallback: %d %+v", rec.Code, got)
	}

	path := filepath.Join(t.TempDir(), "colors.toml")
	if err := os.WriteFile(path, []byte("background = \"url(javascript:bad)\"\naccent = \"#123456\"\n"), 0o644); err != nil {
		t.Fatal(err)
	}
	srv.themePath = path
	rec = do(t, srv, "GET", "/api/theme", nil, "")
	if err := json.Unmarshal(rec.Body.Bytes(), &got); err != nil {
		t.Fatal(err)
	}
	if got.Source != "default" {
		t.Fatalf("unsafe/incomplete theme must fall back: %+v", got)
	}
}
