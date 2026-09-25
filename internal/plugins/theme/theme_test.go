package theme

import (
	"path/filepath"
	"strings"
	"testing"
)

// Ported from the gateway killer tests: the derivation moved into the
// provider behind lain.ui.theme@1 (D-076); the assertions are unchanged.

func TestKillOmarchyThemePath(t *testing.T) {
	t.Setenv("LAIN_OMARCHY_COLORS", "/custom/colors.toml")
	if got := OmarchyPath(); got != "/custom/colors.toml" {
		t.Fatalf("env override: %q", got)
	}
	t.Setenv("LAIN_OMARCHY_COLORS", "   ")
	got := OmarchyPath()
	if got == "/custom/colors.toml" || (got != "" && !strings.HasSuffix(got, filepath.Join("theme", "colors.toml"))) {
		t.Fatalf("blank env must fall back to the default path: %q", got)
	}
}

func TestKillOmarchyThemePathHomeFallback(t *testing.T) {
	home := t.TempDir()
	t.Setenv("LAIN_OMARCHY_COLORS", "")
	t.Setenv("HOME", home)
	want := filepath.Join(home, ".local", "state", "omarchy", "current", "theme", "colors.toml")
	if got := OmarchyPath(); got != want {
		t.Fatalf("default path: %q want %q", got, want)
	}
}

func TestKillThemeContrastMath(t *testing.T) {
	if got := ContrastRatio("#000000", "#ffffff"); got < 20.9 || got > 21.1 {
		t.Fatalf("black/white contrast: %v", got)
	}
	// Symmetric: swapping args must not change the ratio.
	a, b := "#123456", "#fedcba"
	if ContrastRatio(a, b) != ContrastRatio(b, a) {
		t.Fatal("contrastRatio must be symmetric")
	}
	if contrastColor("#000000") != "#ffffff" || contrastColor("#ffffff") != "#000000" {
		t.Fatal("contrastColor must pick the farther pole")
	}
	// 0x0a sits below the sRGB linearisation knee, 0x0b above it.
	if hexLuminance("#0a0a0a") >= hexLuminance("#0b0b0b") {
		t.Fatal("the sRGB knee must stay monotonic")
	}
}

func TestKillThemeParseModes(t *testing.T) {
	// mode validation: only dark/light are accepted; junk is ignored.
	const base = "background = \"#101010\"\nforeground = \"#f0f0f0\"\naccent = \"#6699cc\"\n"
	p, ok := ParseOmarchy(strings.NewReader("mode = \"dark\"\n" + base))
	if !ok || p.Mode != "dark" {
		t.Fatalf("dark mode parse: %+v %v", p, ok)
	}
	p, ok = ParseOmarchy(strings.NewReader("mode = \"sepia\"\n" + base))
	if !ok || p.Mode != "dark" {
		t.Fatalf("invalid mode must fall back to dark: %q %v", p.Mode, ok)
	}
	// Inline comments strip only for unquoted non-# values: `dark # x`
	// parses as dark, while `#101010 # x` keeps its leading #.
	p, ok = ParseOmarchy(strings.NewReader("mode = dark # a comment\n" + base))
	if !ok || p.Mode != "dark" {
		t.Fatalf("inline comment on a bare value: %q %v", p.Mode, ok)
	}
}

func TestKillThemePaletteFixup(t *testing.T) {
	// A collapsed palette (everything black) forces every fixup branch:
	// SurfaceActive must separate from Surface, Muted must clear the
	// text floor, Line must be visible.
	p := ApplyLegibilityFloor(Palette{
		Background: "#000000", Surface: "#000000", SurfaceActive: "#000000",
		Foreground: "#ffffff", Muted: "#000000", Line: "#000000",
		Accent: "#000000",
	})
	if ContrastRatio(p.SurfaceActive, p.Surface) < MinLineContrast-0.05 {
		t.Fatalf("surface_active must read as a fill: %q on %q", p.SurfaceActive, p.Surface)
	}
	if ContrastRatio(p.Muted, p.SurfaceActive) < MinTextContrast-0.05 {
		t.Fatalf("muted must clear the text floor: %q on %q", p.Muted, p.SurfaceActive)
	}
	if ContrastRatio(p.Line, p.Surface) < MinLineContrast-0.05 {
		t.Fatalf("line must be visible: %q on %q", p.Line, p.Surface)
	}
}

func TestKillThemeCommentStrip(t *testing.T) {
	const base = "background = \"#101010\"\nforeground = \"#f0f0f0\"\n"
	// A leading '#' at index 0 is the colour itself, never a comment.
	p, ok := ParseOmarchy(strings.NewReader(base + "accent = #aabbcc\n"))
	if !ok || p.Accent != "#aabbcc" {
		t.Fatalf("unquoted hex value must survive: %q %v", p.Accent, ok)
	}
	// An inline comment after a bare word strips (i>0 branch).
	p, ok = ParseOmarchy(strings.NewReader("mode = light # trailing\n" + base + "accent = \"#aabbcc\"\n"))
	if !ok || p.Mode != "light" {
		t.Fatalf("commented bare mode must parse to light: %q %v", p.Mode, ok)
	}
	// Quotes protect an inner '#': a quoted colour keeps its leading '#'
	// instead of being truncated to a bare quote.
	for _, q := range []string{`'`, `"`} {
		p, ok = ParseOmarchy(strings.NewReader(base + "accent = " + q + "#112233" + q + "\n"))
		if !ok || p.Accent != "#112233" {
			t.Fatalf("quoted colour: %q %v", p.Accent, ok)
		}
	}
}
