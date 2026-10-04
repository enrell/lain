// Package bencode reads and writes BitTorrent's bencoding (BEP 3).
// Values decode to int64, string (byte strings are Go strings, which
// hold arbitrary bytes), []any and map[string]any. Input is untrusted:
// nesting depth, element counts and string lengths are bounded, and
// every error is returned, never panicked.
package bencode

import (
	"bytes"
	"errors"
	"fmt"
	"sort"
	"strconv"
)

// Limits keep hostile input from exhausting memory or the stack.
const (
	MaxDepth = 64
	MaxItems = 1 << 20
)

// ErrSyntax wraps every decoding failure.
var ErrSyntax = errors.New("bencode: syntax error")

func syntax(at int, format string, args ...any) error {
	return fmt.Errorf("%w at %d: %s", ErrSyntax, at, fmt.Sprintf(format, args...))
}

type decoder struct {
	data  []byte
	pos   int
	items int
	// raw records the byte span of each top-level dict value, so callers
	// can hash the exact "info" bytes (the infohash is over the original
	// encoding, not a re-encoding).
	raw map[string][]byte
}

// Decode parses exactly one value; trailing bytes are an error.
func Decode(data []byte) (any, error) {
	v, rest, err := DecodePrefix(data)
	if err != nil {
		return nil, err
	}
	if len(rest) != 0 {
		return nil, syntax(len(data)-len(rest), "trailing data")
	}
	return v, nil
}

// DecodePrefix parses one value and returns the bytes after it. The
// ut_metadata extension sends a dict followed by raw piece bytes.
func DecodePrefix(data []byte) (any, []byte, error) {
	d := &decoder{data: data}
	v, err := d.value(0)
	if err != nil {
		return nil, nil, err
	}
	return v, data[d.pos:], nil
}

// DecodeDict parses a top-level dictionary and also returns the raw
// encoded bytes of each of its values.
func DecodeDict(data []byte) (map[string]any, map[string][]byte, error) {
	d := &decoder{data: data, raw: map[string][]byte{}}
	v, err := d.value(0)
	if err != nil {
		return nil, nil, err
	}
	if d.pos != len(data) {
		return nil, nil, syntax(d.pos, "trailing data")
	}
	m, ok := v.(map[string]any)
	if !ok {
		return nil, nil, syntax(0, "not a dictionary")
	}
	return m, d.raw, nil
}

func (d *decoder) value(depth int) (any, error) {
	if depth > MaxDepth {
		return nil, syntax(d.pos, "nesting too deep")
	}
	if d.items++; d.items > MaxItems {
		return nil, syntax(d.pos, "too many items")
	}
	if d.pos >= len(d.data) {
		return nil, syntax(d.pos, "unexpected end")
	}
	switch c := d.data[d.pos]; {
	case c == 'i':
		return d.integer()
	case c == 'l':
		d.pos++
		list := []any{}
		for {
			if d.pos >= len(d.data) {
				return nil, syntax(d.pos, "unterminated list")
			}
			if d.data[d.pos] == 'e' {
				d.pos++
				return list, nil
			}
			v, err := d.value(depth + 1)
			if err != nil {
				return nil, err
			}
			list = append(list, v)
		}
	case c == 'd':
		d.pos++
		m := map[string]any{}
		for {
			if d.pos >= len(d.data) {
				return nil, syntax(d.pos, "unterminated dictionary")
			}
			if d.data[d.pos] == 'e' {
				d.pos++
				return m, nil
			}
			if d.data[d.pos] < '0' || d.data[d.pos] > '9' {
				return nil, syntax(d.pos, "dictionary key must be a string")
			}
			k, err := d.str()
			if err != nil {
				return nil, err
			}
			start := d.pos
			v, err := d.value(depth + 1)
			if err != nil {
				return nil, err
			}
			if depth == 0 && d.raw != nil {
				d.raw[k] = d.data[start:d.pos]
			}
			m[k] = v
		}
	case c >= '0' && c <= '9':
		return d.str()
	default:
		return nil, syntax(d.pos, "unexpected byte %q", c)
	}
}

func (d *decoder) integer() (int64, error) {
	start := d.pos
	end := bytes.IndexByte(d.data[d.pos:], 'e')
	if end < 0 {
		return 0, syntax(start, "unterminated integer")
	}
	s := string(d.data[d.pos+1 : d.pos+end])
	if s == "" || s == "-" || s == "-0" || (len(s) > 1 && s[0] == '0') || (len(s) > 2 && s[0] == '-' && s[1] == '0') {
		return 0, syntax(start, "non-canonical integer %q", s)
	}
	n, err := strconv.ParseInt(s, 10, 64)
	if err != nil {
		return 0, syntax(start, "bad integer %q", s)
	}
	d.pos += end + 1
	return n, nil
}

func (d *decoder) str() (string, error) {
	start := d.pos
	colon := bytes.IndexByte(d.data[d.pos:], ':')
	if colon < 0 || colon > 10 {
		return "", syntax(start, "bad string length")
	}
	n, err := strconv.Atoi(string(d.data[d.pos : d.pos+colon]))
	if err != nil || n < 0 || (colon > 1 && d.data[d.pos] == '0') {
		return "", syntax(start, "bad string length")
	}
	d.pos += colon + 1
	if n > len(d.data)-d.pos {
		return "", syntax(start, "string longer than input")
	}
	s := string(d.data[d.pos : d.pos+n])
	d.pos += n
	return s, nil
}

// Encode writes v canonically: dictionary keys sorted. Accepted types
// are integers, string, []byte, []any, []string, map[string]any and
// RawMessage (pre-encoded bytes).
func Encode(v any) ([]byte, error) {
	var b bytes.Buffer
	if err := encode(&b, v); err != nil {
		return nil, err
	}
	return b.Bytes(), nil
}

// RawMessage is an already-encoded value inserted verbatim.
type RawMessage []byte

func encode(b *bytes.Buffer, v any) error {
	switch x := v.(type) {
	case int:
		fmt.Fprintf(b, "i%de", x)
	case int64:
		fmt.Fprintf(b, "i%de", x)
	case uint16:
		fmt.Fprintf(b, "i%de", x)
	case uint32:
		fmt.Fprintf(b, "i%de", x)
	case string:
		fmt.Fprintf(b, "%d:", len(x))
		b.WriteString(x)
	case []byte:
		fmt.Fprintf(b, "%d:", len(x))
		b.Write(x)
	case RawMessage:
		b.Write(x)
	case []string:
		b.WriteByte('l')
		for _, s := range x {
			fmt.Fprintf(b, "%d:%s", len(s), s)
		}
		b.WriteByte('e')
	case []any:
		b.WriteByte('l')
		for _, e := range x {
			if err := encode(b, e); err != nil {
				return err
			}
		}
		b.WriteByte('e')
	case map[string]any:
		keys := make([]string, 0, len(x))
		for k := range x {
			keys = append(keys, k)
		}
		sort.Strings(keys)
		b.WriteByte('d')
		for _, k := range keys {
			fmt.Fprintf(b, "%d:%s", len(k), k)
			if err := encode(b, x[k]); err != nil {
				return err
			}
		}
		b.WriteByte('e')
	default:
		return fmt.Errorf("bencode: cannot encode %T", v)
	}
	return nil
}

// Helpers for reading decoded dictionaries without type-switch noise.

// Str returns m[k] as a string.
func Str(m map[string]any, k string) (string, bool) {
	s, ok := m[k].(string)
	return s, ok
}

// Int returns m[k] as an int64.
func Int(m map[string]any, k string) (int64, bool) {
	n, ok := m[k].(int64)
	return n, ok
}

// Dict returns m[k] as a dictionary.
func Dict(m map[string]any, k string) (map[string]any, bool) {
	d, ok := m[k].(map[string]any)
	return d, ok
}

// List returns m[k] as a list.
func List(m map[string]any, k string) ([]any, bool) {
	l, ok := m[k].([]any)
	return l, ok
}
