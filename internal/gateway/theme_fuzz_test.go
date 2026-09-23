package gateway

// mutation-clean: gremlins v0.6.0 — package verified 2026-09-23

import (
	"strings"
	"testing"
)

// FuzzParseOmarchyTheme feeds arbitrary CSS-ish text to the theme
// parser. The file is user-supplied; every accepted palette must carry
// only #rrggbb colors in its color fields.
//
// Campaign: go test -fuzz=FuzzParseOmarchyTheme -fuzztime=60s ./internal/gateway/
func FuzzParseOmarchyTheme(f *testing.F) {
	for _, s := range []string{
		"background=#101010\nforeground=#e0e0e0\naccent=#3366ff\n",
		"mode = light\nbackground = #ffffff\nforeground = #000000\naccent = #123456\n",
		"background='#101010'\nforeground=\"#e0e0e0\"\naccent=#3366ff\n",
		"background = #101010 # trailing comment\nforeground=#e0e0e0\naccent=#3366ff\n",
		"background=#fff\nforeground=#e0e0e0\naccent=#3366ff\n",
		"mode=sepia\nbackground=#101010\nforeground=#e0e0e0\naccent=#3366ff\n",
		"no equals sign\n=novalue\nkey=\n",
		"accent = url(http://x)\nbackground=#101010\nforeground=#e0e0e0\n",
		"accent=#3366ff\nbackground=#101010\nforeground=#e0e0e0\nmode=light # comment\n",
		"",
		"\x00\x01\x02",
		strings.Repeat("color1=#aabbcc\n", 100),
	} {
		f.Add(s)
	}
	f.Fuzz(func(t *testing.T, css string) {
		p, ok := parseOmarchyTheme(strings.NewReader(css))
		if !ok {
			return
		}
		for name, c := range map[string]string{
			"background": p.Background, "surface": p.Surface,
			"surface_hover": p.SurfaceHover, "surface_active": p.SurfaceActive,
			"foreground": p.Foreground, "muted": p.Muted, "line": p.Line,
			"accent": p.Accent, "accent_hover": p.AccentHover,
			"accent_foreground": p.AccentForeground,
			"danger":            p.Danger, "danger_foreground": p.DangerForeground,
			"success": p.Success, "warning": p.Warning,
		} {
			if !hexColor.MatchString(c) {
				t.Fatalf("palette field %s = %q is not #rrggbb (input %q)", name, c, css)
			}
		}
		if p.Mode != "dark" && p.Mode != "light" {
			t.Fatalf("mode %q is not dark|light", p.Mode)
		}
	})
}
