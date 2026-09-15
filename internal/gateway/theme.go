package gateway

import (
	"bufio"
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
				palette = parsed
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

func contrastColor(color string) string {
	component := func(offset int) float64 {
		n, _ := strconv.ParseUint(color[offset:offset+2], 16, 8)
		v := float64(n) / 255
		if v <= 0.04045 {
			return v / 12.92
		}
		return math.Pow((v+0.055)/1.055, 2.4)
	}
	luminance := 0.2126*component(1) + 0.7152*component(3) + 0.0722*component(5)
	blackContrast := (luminance + 0.05) / 0.05
	whiteContrast := 1.05 / (luminance + 0.05)
	if blackContrast >= whiteContrast {
		return "#000000"
	}
	return "#ffffff"
}
