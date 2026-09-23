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

// D-037: whatever the host theme collapsed, the derivation must still hand the
// browser roles that can carry text and draw a line.
func TestThemeFloorKeepsCollapsedOperatorPaletteLegible(t *testing.T) {
	cases := []struct {
		name   string
		colors string
	}{
		{
			// The host this run found: selection, color8 and the muted
			// fallback are one swatch, so muted/line/surface-active collapse.
			name: "selection collapses the neutral roles",
			colors: `mode = "dark"
background = "#0c0b0c"
dark_background = "#090809"
lighter_background = "#0c0b0c"
foreground = "#FAFCFB"
muted = "#584e51"
selection = "#584e51"
accent = "#b59790"
red = "#c38b7b"
green = "#87a9b0"
yellow = "#6b5e73"
`,
		},
		{
			name: "every neutral role is one mid grey",
			colors: `mode = "dark"
background = "#101010"
dark_background = "#101010"
lighter_background = "#101010"
foreground = "#eeeeee"
muted = "#808080"
selection = "#808080"
accent = "#4488cc"
red = "#cc4444"
green = "#44cc44"
yellow = "#cccc44"
`,
		},
		{
			// The host this run found: `selection` is a mid-tone highlight, so
			// surface-active was a fill no text could sit on (even the palette's
			// own foreground stops at 2.2:1 on it) and muted landed on it as soon
			// as it cleared the body floor, at 1.0:1. Its shade keys are named
			// differently, so surface falls back to the background here.
			name: "selection is a mid-tone highlight",
			colors: `mode = "dark"
background = "#151623"
foreground = "#bac5cd"
muted = "#4b4d53"
selection = "#9a778a"
accent = "#9a778a"
red = "#9a778a"
green = "#b4c9ab"
yellow = "#8f7f62"
`,
		},
		{
			// Secondary text that is simply too dim for body copy.
			name: "muted is below the body floor",
			colors: `mode = "light"
background = "#FFFCF0"
dark_background = "#F2EFE4"
lighter_background = "#E6E4D9"
foreground = "#100F0F"
muted = "#878580"
selection = "#CECDC3"
accent = "#205EA6"
red = "#D14D41"
green = "#879A39"
yellow = "#D0A215"
`,
		},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			srv := testServer(t)
			path := filepath.Join(t.TempDir(), "colors.toml")
			if err := os.WriteFile(path, []byte(tc.colors), 0o644); err != nil {
				t.Fatal(err)
			}
			srv.themePath = path
			rec := do(t, srv, "GET", "/api/theme", nil, "")
			var got themePalette
			if err := json.Unmarshal(rec.Body.Bytes(), &got); err != nil {
				t.Fatal(err)
			}
			if got.Source != "omarchy" {
				t.Fatalf("expected the host palette, got %+v", got)
			}
			if r := contrastRatio(got.Muted, got.Background); r < minTextContrast {
				t.Fatalf("muted %s on background %s = %.2f:1, want >= %.1f", got.Muted, got.Background, r, minTextContrast)
			}
			if r := contrastRatio(got.Muted, got.SurfaceActive); r < minTextContrast {
				t.Fatalf("muted %s on surface_active %s = %.2f:1, want >= %.1f (neutral badge text on its fill)", got.Muted, got.SurfaceActive, r, minTextContrast)
			}
			if r := contrastRatio(got.SurfaceActive, got.Surface); r < minLineContrast {
				t.Fatalf("surface_active %s on surface %s = %.2f:1, want >= %.2f (a fill must read as a fill)", got.SurfaceActive, got.Surface, r, minLineContrast)
			}
			if r := contrastRatio(got.Line, got.Surface); r < minLineContrast {
				t.Fatalf("line %s on surface %s = %.2f:1, want >= %.2f", got.Line, got.Surface, r, minLineContrast)
			}
			if r := contrastRatio(got.Line, got.SurfaceActive); r < minLineContrast {
				t.Fatalf("line %s on surface_active %s = %.2f:1, want >= %.2f", got.Line, got.SurfaceActive, r, minLineContrast)
			}
		})
	}
}

// A host theme that already separates its roles is served unchanged: the floor
// repairs collapsed palettes, it does not restyle good ones.
func TestThemeFloorLeavesSeparatedPaletteAlone(t *testing.T) {
	srv := testServer(t)
	path := filepath.Join(t.TempDir(), "colors.toml")
	colors := `mode = "dark"
background = "#0c0b0c"
dark_background = "#141216"
lighter_background = "#1c1a1e"
foreground = "#FAFCFB"
muted = "#a8a2a4"
selection = "#3a3438"
accent = "#b59790"
red = "#c38b7b"
green = "#87a9b0"
yellow = "#6b5e73"
`
	if err := os.WriteFile(path, []byte(colors), 0o644); err != nil {
		t.Fatal(err)
	}
	srv.themePath = path
	rec := do(t, srv, "GET", "/api/theme", nil, "")
	var got themePalette
	if err := json.Unmarshal(rec.Body.Bytes(), &got); err != nil {
		t.Fatal(err)
	}
	if got.Muted != "#a8a2a4" || got.SurfaceActive != "#3a3438" {
		t.Fatalf("separated palette was restyled: %+v", got)
	}
	if r := contrastRatio(got.Muted, got.Background); r < minTextContrast {
		t.Fatalf("muted/background = %.2f:1", r)
	}
	if r := contrastRatio(got.Muted, got.SurfaceActive); r < minTextContrast {
		t.Fatalf("muted/surface_active = %.2f:1", r)
	}
	if r := contrastRatio(got.Line, got.SurfaceActive); r < minLineContrast {
		t.Fatalf("line/surface_active = %.2f:1", r)
	}
}
