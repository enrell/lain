package gateway

import (
	"bufio"
	"fmt"
	"io"
	"math"
	"net/http"
	"os"
	"path/filepath"
	"regexp"
	"strconv"
	"strings"
)

const maxThemeBytes = 64 << 10

var hexColor = regexp.MustCompile(`^#[0-9A-Fa-f]{6}$`)

// themePalette is already semantic: the browser can assign these values to
// its CSS tokens without receiving a local path or untrusted theme content.
type themePalette struct {
	Source           string `json:"source"`
	Mode             string `json:"mode"`
	Background       string `json:"background"`
	Surface          string `json:"surface"`
	SurfaceHover     string `json:"surface_hover"`
	SurfaceActive    string `json:"surface_active"`
	Foreground       string `json:"foreground"`
	Muted            string `json:"muted"`
	Line             string `json:"line"`
	Accent           string `json:"accent"`
	AccentHover      string `json:"accent_hover"`
	AccentForeground string `json:"accent_foreground"`
	Danger           string `json:"danger"`
	DangerForeground string `json:"danger_foreground"`
	Success          string `json:"success"`
	Warning          string `json:"warning"`
}

func defaultTheme() themePalette {
	return themePalette{
		Source:           "default",
		Mode:             "dark",
		Background:       "#08090c",
		Surface:          "#101218",
		SurfaceHover:     "#171a22",
		SurfaceActive:    "#1e222c",
		Foreground:       "#e7e9f0",
		Muted:            "#8a90a3",
		Line:             "#1f2430",
		Accent:           "#5ce1c4",
		AccentHover:      "#7af0d6",
		AccentForeground: "#05231d",
		Danger:           "#ff6b81",
		DangerForeground: "#2b060d",
		Success:          "#6ee7a8",
		Warning:          "#ffcf5c",
	}
}

func omarchyThemePath() string {
	if path := strings.TrimSpace(os.Getenv("LAIN_OMARCHY_COLORS")); path != "" {
		return path
	}
	home, err := os.UserHomeDir()
	if err != nil || home == "" {
		return ""
	}
	return filepath.Join(home, ".local", "state", "omarchy", "current", "theme", "colors.toml")
}

func (s *Server) handleTheme(w http.ResponseWriter, _ *http.Request) {
	w.Header().Set("Cache-Control", "no-store")
	palette := defaultTheme()
	if path := s.themePath; path != "" {
		if f, err := os.Open(path); err == nil {
			if parsed, ok := parseOmarchyTheme(io.LimitReader(f, maxThemeBytes)); ok {
				// D-037: a host theme can map several roles onto one swatch;
				// the derivation, not the components, owns the legibility floor.
				// The built-in fallback keeps its curated values.
				palette = applyLegibilityFloor(parsed)
			}
			_ = f.Close()
		}
	}
	writeJSON(w, http.StatusOK, palette)
}

func parseOmarchyTheme(r io.Reader) (themePalette, bool) {
	values := make(map[string]string)
	scanner := bufio.NewScanner(r)
	for scanner.Scan() {
		key, value, ok := strings.Cut(scanner.Text(), "=")
		if !ok {
			continue
		}
		key = strings.TrimSpace(key)
		value = strings.TrimSpace(value)
		if i := strings.IndexByte(value, '#'); i > 0 && value[0] != '"' && value[0] != '\'' {
			value = strings.TrimSpace(value[:i])
		}
		value = strings.Trim(value, `"'`)
		if key == "mode" {
			if value == "dark" || value == "light" {
				values[key] = value
			}
			continue
		}
		if hexColor.MatchString(value) {
			values[key] = strings.ToLower(value)
		}
	}
	if err := scanner.Err(); err != nil {
		return themePalette{}, false
	}

	background := firstColor(values, "background", "color0")
	foreground := firstColor(values, "foreground", "color7")
	accent := firstColor(values, "accent", "color4")
	if background == "" || foreground == "" || accent == "" {
		return themePalette{}, false
	}

	fallback := defaultTheme()
	mode := values["mode"]
	if mode == "" {
		mode = "dark"
	}
	surface := colorOr(firstColor(values, "dark_background"), background)
	surfaceHover := colorOr(firstColor(values, "lighter_background", "selection"), surface)
	surfaceActive := colorOr(firstColor(values, "selection", "lighter_background"), surfaceHover)
	muted := colorOr(firstColor(values, "muted", "color8", "dark_foreground"), foreground)
	line := colorOr(firstColor(values, "selection", "muted", "color8"), muted)
	accentHover := colorOr(firstColor(values, "bright_blue"), accent)
	danger := colorOr(firstColor(values, "red", "color1"), fallback.Danger)
	success := colorOr(firstColor(values, "green", "color2"), fallback.Success)
	warning := colorOr(firstColor(values, "yellow", "color3"), fallback.Warning)

	return themePalette{
		Source:           "omarchy",
		Mode:             mode,
		Background:       background,
		Surface:          surface,
		SurfaceHover:     surfaceHover,
		SurfaceActive:    surfaceActive,
		Foreground:       foreground,
		Muted:            muted,
		Line:             line,
		Accent:           accent,
		AccentHover:      accentHover,
		AccentForeground: contrastColor(accent),
		Danger:           danger,
		DangerForeground: contrastColor(danger),
		Success:          success,
		Warning:          warning,
	}, true
}

func firstColor(values map[string]string, keys ...string) string {
	for _, key := range keys {
		if value := values[key]; value != "" {
			return value
		}
	}
	return ""
}

func colorOr(value, fallback string) string {
	if value != "" {
		return value
	}
	return fallback
}

// Legibility floor for operator palettes (D-037). A host Omarchy theme can
// collapse several semantic roles onto one swatch, which renders neutral
// badges, separators and secondary text invisible; the endpoint must still
// hand the browser values that can carry text and draw a line.
const (
	// minTextContrast is the body-text floor secondary text must clear, both
	// against the page background and against the fill it is painted on
	// (neutral badges, avatar monograms).
	minTextContrast = 4.5
	// minLineContrast is how far a separator must stand out from the surface
	// it borders.
	minLineContrast = 1.25
)

// applyLegibilityFloor returns p with distinct, contrast-checked roles. It only
// moves muted, surface-active and line, and only when they violate the floor,
// so a host theme that already separates them is served unchanged.
func applyLegibilityFloor(p themePalette) themePalette {
	// 1. The fill. Operator themes hand this role their text-selection
	// highlight, and a mid tone carries no text at all: on a #9a778a fill even
	// the palette's own foreground stops at 2.2:1, so a neutral badge stays
	// unreadable whichever text role is chosen. When neither secondary nor
	// primary text clears the floor on the host value, re-derive the fill as
	// the smallest lift off the surface — which is what the role means and what
	// the built-in palette does — so badges, switch tracks and skeletons stay a
	// fill instead of a wash.
	if contrastRatio(p.SurfaceActive, p.Muted) < minTextContrast &&
		contrastRatio(p.SurfaceActive, p.Foreground) < minTextContrast {
		p.SurfaceActive = p.Surface
	}
	// 2. ...and it must read as a fill and not as the surface it sits on.
	p.SurfaceActive = blendTo(p.SurfaceActive, p.Surface, p.Foreground, minLineContrast)
	// 3. Secondary text clears the body floor against the page and against the
	// fill it is painted on (neutral badges, avatar monograms).
	p.Muted = blendTo(p.Muted, p.SurfaceActive, p.Foreground, minTextContrast)
	p.Muted = blendTo(p.Muted, p.Background, p.Foreground, minTextContrast)
	// 4. Separators must be visible against the surfaces they border.
	p.Line = blendTo(p.Line, p.Surface, p.Foreground, minLineContrast)
	p.Line = blendTo(p.Line, p.SurfaceActive, p.Foreground, minLineContrast)
	return p
}

// blendTo blends c toward target until it clears min contrast against other, and
// always moves. A palette whose two roles meet at a knife edge (muted exactly
// at the floor on the page, so the fill can only reach 4.35:1 on the way to the
// background) used to fall short of the floor by a rounding step and be
// discarded, which served the very collapse D-037 forbids; the pole is now the
// last resort, because it is the most separation those two roles allow.
func blendTo(c, other, target string, min float64) string {
	if contrastRatio(c, other) >= min {
		return c
	}
	for range 256 {
		next := mixHex(c, target, 0.08)
		if next == c {
			break
		}
		c = next
		if contrastRatio(c, other) >= min {
			return c
		}
	}
	return target
}

// mixHex returns a and b blended by t (0 keeps a, 1 keeps b).
func mixHex(a, b string, t float64) string {
	mix := func(offset int) int {
		x, _ := strconv.ParseUint(a[offset:offset+2], 16, 8)
		y, _ := strconv.ParseUint(b[offset:offset+2], 16, 8)
		return int(math.Round(float64(x) + (float64(y)-float64(x))*t))
	}
	return fmt.Sprintf("#%02x%02x%02x", mix(1), mix(3), mix(5))
}

// hexLuminance is the WCAG relative luminance of a #rrggbb colour.
func hexLuminance(color string) float64 {
	component := func(offset int) float64 {
		n, _ := strconv.ParseUint(color[offset:offset+2], 16, 8)
		v := float64(n) / 255
		if v <= 0.04045 {
			return v / 12.92
		}
		return math.Pow((v+0.055)/1.055, 2.4)
	}
	return 0.2126*component(1) + 0.7152*component(3) + 0.0722*component(5)
}

// contrastRatio is the WCAG contrast ratio between two #rrggbb colours.
func contrastRatio(a, b string) float64 {
	la, lb := hexLuminance(a), hexLuminance(b)
	if la < lb {
		la, lb = lb, la
	}
	return (la + 0.05) / (lb + 0.05)
}

func contrastColor(color string) string {
	luminance := hexLuminance(color)
	blackContrast := (luminance + 0.05) / 0.05
	whiteContrast := 1.05 / (luminance + 0.05)
	if blackContrast >= whiteContrast {
		return "#000000"
	}
	return "#ffffff"
}
