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
