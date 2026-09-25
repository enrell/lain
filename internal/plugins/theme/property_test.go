package theme

import (
	"math/rand"
	"reflect"
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
		return ContrastRatio(string(c), got) >= ContrastRatio(string(c), other)
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
		if ContrastRatio(c, other) >= min {
			return out == c
		}
		return ContrastRatio(out, other) >= min || out == tgt
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
		ab, ba := ContrastRatio(string(a), string(b)), ContrastRatio(string(b), string(a))
		aa := ContrastRatio(string(a), string(a))
		return ab == ba && aa == 1 && ab >= 1 && ab <= 21.01
	}, &quick.Config{MaxCount: 500})
	if err != nil {
		t.Error(err)
	}
}
