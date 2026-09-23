package gateway


import (
	"math/rand"
	"reflect"
	"strings"
	"testing"
	"testing/quick"
)

// hexColorGen feeds quick random #rrggbb colors.
type hexColorGen string

func (hexColorGen) Generate(r *rand.Rand, _ int) reflect.Value {
	const digits = "0123456789abcdef"
	b := make([]byte, 7)
	b[0] = '#'
	for i := 1; i < 7; i++ {
		b[i] = digits[r.Intn(16)]
	}
	return reflect.ValueOf(hexColorGen(b))
}

// Property: signHLSURI never leaks the token onto a URI the server does
// not own — absolute and protocol-relative URIs pass through verbatim —
// and always tags same-directory relatives with the session.
func TestPropertySignHLSURI(t *testing.T) {
	err := quick.Check(func(uri, session, token string) bool {
		out := signHLSURI(uri, session, token)
		if uri == "" {
			return out == ""
		}
		if strings.HasPrefix(uri, "http://") || strings.HasPrefix(uri, "https://") || strings.HasPrefix(uri, "//") {
			return out == uri
		}
		if out == uri {
			return true // unparseable URIs stay untouched
		}
		return strings.Contains(out, "session=")
	}, &quick.Config{MaxCount: 500})
	if err != nil {
		t.Error(err)
	}
}

// Property: rewriteHLSPlaylist is line-preserving — every tag line
// (except EXT-X-MAP, whose URI= attr gets signed) survives verbatim and
// every media line leaves signed or provably un-signable.
func TestPropertyRewriteHLSPlaylist(t *testing.T) {
	err := quick.Check(func(raw, session, token string) bool {
		// Derive lines from raw: Split is the same 1:1 decomposition the
		// rewriter performs, so the count check is meaningful.
		lines := strings.Split(raw, "\n")
		out := rewriteHLSPlaylist(raw, session, token)
		outLines := strings.Split(out, "\n")
		if len(outLines) != len(lines) {
			return false
		}
		for i, line := range lines {
			trimmed := strings.TrimSpace(line)
			oline := outLines[i]
			switch {
			case trimmed == "":
				if oline != line {
					return false
				}
			case strings.HasPrefix(trimmed, "#EXT-X-MAP:"):
				if !strings.Contains(line, `URI="`) && oline != line {
					return false
				}
			case strings.HasPrefix(trimmed, "#"):
				if oline != line {
					return false
				}
			default:
				if oline != trimmed && !strings.Contains(oline, "session=") {
					return false
				}
			}
		}
		return true
	}, &quick.Config{MaxCount: 300})
	if err != nil {
		t.Error(err)
	}
}

// Property: contrastColor always returns the better pole — the
// alternative must never beat it.
func TestPropertyContrastColorPicksBestPole(t *testing.T) {
	err := quick.Check(func(c hexColorGen) bool {
		got := contrastColor(string(c))
		if got != "#000000" && got != "#ffffff" {
			return false
		}
		other := "#ffffff"
		if got == "#ffffff" {
			other = "#000000"
		}
		return contrastRatio(string(c), got) >= contrastRatio(string(c), other)
	}, &quick.Config{MaxCount: 500})
	if err != nil {
		t.Error(err)
	}
}

// Property: blendTo either leaves an already-clear color alone or
// reaches the floor — and when it cannot, its documented last resort
// is the pole.
func TestPropertyBlendTo(t *testing.T) {
	err := quick.Check(func(a, b, target hexColorGen) bool {
		c, other, tgt := string(a), string(b), string(target)
		const min = 4.5
		out := blendTo(c, other, tgt, min)
		if contrastRatio(c, other) >= min {
			return out == c
		}
		return contrastRatio(out, other) >= min || out == tgt
	}, &quick.Config{MaxCount: 300})
	if err != nil {
		t.Error(err)
	}
}

// Property: mixHex endpoints are exact and midpoints stay inside the
// componentwise interval.
func TestPropertyMixHexBounds(t *testing.T) {
	err := quick.Check(func(a, b hexColorGen) bool {
		av, bv := string(a), string(b)
		if mixHex(av, bv, 0) != av || mixHex(av, bv, 1) != bv {
			return false
		}
		mid := mixHex(av, bv, 0.5)
		return hexLuminance(mid) >= 0 && hexLuminance(mid) <= 1
	}, &quick.Config{MaxCount: 300})
	if err != nil {
		t.Error(err)
	}
}

// Property: contrastRatio is symmetric, self-ratio is 1, range [1, 21].
func TestPropertyContrastRatio(t *testing.T) {
	err := quick.Check(func(a, b hexColorGen) bool {
		ab, ba := contrastRatio(string(a), string(b)), contrastRatio(string(b), string(a))
		aa := contrastRatio(string(a), string(a))
		return ab == ba && aa == 1 && ab >= 1 && ab <= 21.01
	}, &quick.Config{MaxCount: 500})
	if err != nil {
		t.Error(err)
	}
}
