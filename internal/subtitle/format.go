// Package subtitle reads subtitle files: SRT, WebVTT and ASS/SSA are
// parsed into timed cues and written back as WebVTT, the one format
// browsers play (docs/slices/acquisition.md, A-29). Input is untrusted:
// size and cue counts are bounded and nothing panics. Standard library
// only (A-34).
package subtitle

import (
	"bytes"
	"errors"
	"fmt"
	"strconv"
	"strings"
	"time"
	"unicode/utf16"
	"unicode/utf8"
)

// Bounds on what a subtitle file may hold.
const (
	MaxBytes = 5 << 20
	MaxCues  = 20000
)

// ErrFormat wraps every parse failure.
var ErrFormat = errors.New("subtitle: unreadable")

// Cue is one timed piece of text. Text may carry WebVTT-compatible
// inline tags (<i>, <b>, <u>, <v>); lines are separated by "\n".
type Cue struct {
	Start time.Duration
	End   time.Duration
	Text  string
}

// Parse detects the format by content and returns the cues in file
// order. A file without a single cue is an error.
func Parse(data []byte) ([]Cue, error) {
	if len(data) > MaxBytes {
		return nil, fmt.Errorf("%w: larger than %d bytes", ErrFormat, MaxBytes)
	}
	text := string(DecodeText(data))
	text = strings.ReplaceAll(strings.ReplaceAll(text, "\r\n", "\n"), "\r", "\n")
	var cues []Cue
	switch head := strings.TrimSpace(text); {
	case strings.HasPrefix(head, "WEBVTT"):
		cues = parseTimed(text, true)
	case strings.Contains(strings.ToLower(head), "[events]") || strings.Contains(head, "\nDialogue:") || strings.HasPrefix(head, "Dialogue:"):
		cues = parseASS(text)
	default:
		cues = parseTimed(text, false)
	}
	if len(cues) == 0 {
		return nil, fmt.Errorf("%w: no cues", ErrFormat)
	}
	return cues, nil
}

// parseTimed reads SRT and WebVTT with one line-based state machine,
// tolerant of the usual damage: missing blank lines, dot or comma
// milliseconds, missing hours, stray index lines.
func parseTimed(text string, vtt bool) []Cue {
	lines := strings.Split(text, "\n")
	var out []Cue
	var cur *Cue
	var buf []string
	flush := func() {
		if cur != nil {
			cur.Text = cleanText(strings.Join(buf, "\n"), vtt)
			if cur.Text != "" && len(out) < MaxCues {
				out = append(out, *cur)
			}
		}
		cur, buf = nil, nil
	}
	skipBlock := false
	for i := 0; i < len(lines); i++ {
		line := strings.TrimSpace(lines[i])
		if i == 0 && vtt {
			continue // the WEBVTT header line
		}
		if line == "" {
			flush()
			skipBlock = false
			continue
		}
		if skipBlock {
			continue
		}
		if start, end, ok := parseTiming(line); ok {
			flush()
			if end < start {
				end = start
			}
			cur = &Cue{Start: start, End: end}
			continue
		}
		// A line directly followed by a timing line is a cue identifier
		// (an SRT index, a WebVTT id), never text.
		if i+1 < len(lines) {
			if _, _, ok := parseTiming(strings.TrimSpace(lines[i+1])); ok {
				continue
			}
		}
		if cur == nil {
			if vtt && (strings.HasPrefix(line, "NOTE") || strings.HasPrefix(line, "STYLE") || strings.HasPrefix(line, "REGION")) {
				skipBlock = true
			}
			continue
		}
		buf = append(buf, line)
	}
	flush()
	return out
}

// parseTiming reads "start --> end [settings]".
func parseTiming(line string) (time.Duration, time.Duration, bool) {
	a, b, ok := strings.Cut(line, "-->")
	if !ok {
		return 0, 0, false
	}
	start, ok1 := parseClock(strings.TrimSpace(a))
	fields := strings.Fields(b)
	if len(fields) == 0 {
		return 0, 0, false
	}
	end, ok2 := parseClock(fields[0])
	return start, end, ok1 && ok2
}

// parseClock reads [hh:]mm:ss[.,]fff (1–3 fraction digits).
func parseClock(s string) (time.Duration, bool) {
	s = strings.Replace(s, ",", ".", 1)
	main, frac, _ := strings.Cut(s, ".")
	parts := strings.Split(main, ":")
	if len(parts) < 2 || len(parts) > 3 || len(frac) > 3 {
		return 0, false
	}
	var total int64
	for _, p := range parts {
		if p == "" || len(p) > 3 {
			return 0, false
		}
		n, err := strconv.Atoi(p)
		if err != nil || n < 0 {
			return 0, false
		}
		total = total*60 + int64(n)
	}
	ms := int64(0)
	if frac != "" {
		n, err := strconv.Atoi(frac)
		if err != nil || n < 0 {
			return 0, false
		}
		for i := len(frac); i < 3; i++ {
			n *= 10
		}
		ms = int64(n)
	}
	if total > 100*3600 {
		return 0, false
	}
	return time.Duration(total)*time.Second + time.Duration(ms)*time.Millisecond, true
}

// cleanText drops formatting browsers do not understand (<font>, SSA
// override blocks in SRT) and empty lines.
func cleanText(s string, vtt bool) string {
	if !vtt {
		s = stripTag(s, "font")
		s = stripBraces(s)
	}
	var lines []string
	for _, l := range strings.Split(s, "\n") {
		if l = strings.TrimSpace(l); l != "" {
			lines = append(lines, l)
		}
	}
	return strings.Join(lines, "\n")
}

func stripTag(s, tag string) string {
	var b strings.Builder
	lower := strings.ToLower(s)
	for i := 0; i < len(s); {
		if s[i] == '<' {
			rest := lower[i+1:]
			rest = strings.TrimPrefix(rest, "/")
			if strings.HasPrefix(rest, tag) && (len(rest) == len(tag) || rest[len(tag)] == '>' || rest[len(tag)] == ' ') {
				if end := strings.IndexByte(s[i:], '>'); end >= 0 {
					i += end + 1
					continue
				}
			}
		}
		b.WriteByte(s[i])
		i++
	}
	return b.String()
}

func stripBraces(s string) string {
	var b strings.Builder
	for i := 0; i < len(s); i++ {
		if s[i] == '{' {
			if end := strings.IndexByte(s[i:], '}'); end > 0 && strings.Contains(s[i:i+end], "\\") {
				i += end
				continue
			}
		}
		b.WriteByte(s[i])
	}
	return b.String()
}

// parseASS reads the [Events] section of ASS/SSA: Dialogue lines laid
// out by the section's Format line. Override blocks are dropped; \N is
// a line break, \h a space; vector drawings ({\p1}…{\p0}) are skipped.
func parseASS(text string) []Cue {
	var out []Cue
	fields := []string{"layer", "start", "end", "style", "name", "marginl", "marginr", "marginv", "effect", "text"}
	inEvents := false
	for _, raw := range strings.Split(text, "\n") {
		line := strings.TrimSpace(raw)
		if strings.HasPrefix(line, "[") {
			inEvents = strings.EqualFold(line, "[events]")
			continue
		}
		if strings.HasPrefix(line, "Format:") && inEvents {
			fields = nil
			for _, f := range strings.Split(strings.TrimPrefix(line, "Format:"), ",") {
				fields = append(fields, strings.ToLower(strings.TrimSpace(f)))
			}
			continue
		}
		if !strings.HasPrefix(line, "Dialogue:") || len(out) >= MaxCues {
			continue
		}
		parts := strings.SplitN(strings.TrimSpace(strings.TrimPrefix(line, "Dialogue:")), ",", len(fields))
		if len(parts) != len(fields) {
			continue
		}
		var start, end time.Duration
		var body string
		ok1, ok2 := false, false
		for i, f := range fields {
			switch f {
			case "start":
				start, ok1 = parseClock(strings.TrimSpace(parts[i]))
			case "end":
				end, ok2 = parseClock(strings.TrimSpace(parts[i]))
			case "text":
				body = parts[i]
			}
		}
		if !ok1 || !ok2 {
			continue
		}
		if end < start {
			end = start
		}
		if t := assText(body); t != "" {
			out = append(out, Cue{Start: start, End: end, Text: t})
		}
	}
	return out
}

func assText(s string) string {
	var b strings.Builder
	drawing := false
	for i := 0; i < len(s); i++ {
		switch {
		case s[i] == '{':
			end := strings.IndexByte(s[i:], '}')
			if end < 0 {
				end = len(s) - i - 1
			}
			block := s[i : i+end+1]
			if idx := strings.Index(block, `\p`); idx >= 0 && idx+2 < len(block) {
				drawing = block[idx+2] != '0'
			}
			i += end
		case s[i] == '\\' && i+1 < len(s) && (s[i+1] == 'N' || s[i+1] == 'n'):
			if !drawing {
				b.WriteByte('\n')
			}
			i++
		case s[i] == '\\' && i+1 < len(s) && s[i+1] == 'h':
			if !drawing {
				b.WriteByte(' ')
			}
			i++
		default:
			if !drawing {
				b.WriteByte(s[i])
			}
		}
	}
	return cleanText(b.String(), true)
}

// WebVTT renders cues as a WebVTT document. "&" and "-->" in text are
// escaped so a cue can never break the file's structure.
func WebVTT(cues []Cue) []byte {
	var b bytes.Buffer
	b.WriteString("WEBVTT\n\n")
	for _, c := range cues {
		fmt.Fprintf(&b, "%s --> %s\n", clock(c.Start), clock(c.End))
		text := strings.ReplaceAll(c.Text, "&", "&amp;")
		text = strings.ReplaceAll(text, "-->", "--&gt;")
		var lines []string
		for _, l := range strings.Split(text, "\n") {
			if l = strings.TrimSpace(l); l != "" {
				lines = append(lines, l)
			}
		}
		b.WriteString(strings.Join(lines, "\n"))
		b.WriteString("\n\n")
	}
	return b.Bytes()
}

func clock(d time.Duration) string {
	if d < 0 {
		d = 0
	}
	ms := d.Milliseconds()
	return fmt.Sprintf("%02d:%02d:%02d.%03d", ms/3600000, ms/60000%60, ms/1000%60, ms%1000)
}

// cp1252 maps the 0x80–0x9F range of Windows-1252 (the rest of the
// byte range is Latin-1, identical to Unicode).
var cp1252 = [32]rune{
	'€', 0xFFFD, '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', 0xFFFD, 'Ž', 0xFFFD,
	0xFFFD, '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', 0xFFFD, 'ž', 'Ÿ',
}

// DecodeText returns UTF-8: a UTF-8 BOM is dropped, UTF-16 with a BOM
// is decoded, valid UTF-8 passes through and anything else is read as
// Windows-1252 (A-34).
func DecodeText(b []byte) []byte {
	switch {
	case bytes.HasPrefix(b, []byte{0xEF, 0xBB, 0xBF}):
		b = b[3:]
	case len(b) >= 2 && (b[0] == 0xFF && b[1] == 0xFE || b[0] == 0xFE && b[1] == 0xFF):
		le := b[0] == 0xFF
		b = b[2:]
		u := make([]uint16, len(b)/2)
		for i := range u {
			if le {
				u[i] = uint16(b[2*i]) | uint16(b[2*i+1])<<8
			} else {
				u[i] = uint16(b[2*i])<<8 | uint16(b[2*i+1])
			}
		}
		return []byte(string(utf16.Decode(u)))
	}
	if utf8.Valid(b) {
		return b
	}
	out := make([]rune, 0, len(b))
	for _, c := range b {
		switch {
		case c < 0x80 || c >= 0xA0:
			out = append(out, rune(c))
		default:
			out = append(out, cp1252[c-0x80])
		}
	}
	return []byte(string(out))
}

// Stats summarizes cue timing for the sync check (A-33).
type Stats struct {
	Cues  int
	First time.Duration
	Last  time.Duration
}

// Measure returns the earliest start and latest end.
func Measure(cues []Cue) Stats {
	st := Stats{Cues: len(cues)}
	for i, c := range cues {
		if i == 0 || c.Start < st.First {
			st.First = c.Start
		}
		if c.End > st.Last {
			st.Last = c.End
		}
	}
	return st
}
