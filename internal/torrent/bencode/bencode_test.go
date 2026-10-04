package bencode

import (
	"bytes"
	"errors"
	"reflect"
	"strings"
	"testing"
)

func TestRoundTrip(t *testing.T) {
	v := map[string]any{
		"announce": "http://tracker-exemplo.invalid/announce",
		"info": map[string]any{
			"name":         "a\x00b",
			"piece length": int64(16384),
			"files":        []any{map[string]any{"length": int64(3), "path": []any{"x", "y"}}},
		},
		"neg": int64(-42),
	}
	raw, err := Encode(v)
	if err != nil {
		t.Fatal(err)
	}
	got, err := Decode(raw)
	if err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(got, v) {
		t.Fatalf("round trip:\n got %#v\nwant %#v", got, v)
	}
	again, _ := Encode(got)
	if !bytes.Equal(raw, again) {
		t.Fatal("encoding is not canonical")
	}
}

func TestDecodeDictKeepsRawValues(t *testing.T) {
	in := []byte("d4:infod4:name3:abc6:lengthi5ee3:zzzi1ee")
	m, raw, err := DecodeDict(in)
	if err != nil {
		t.Fatal(err)
	}
	if string(raw["info"]) != "d4:name3:abc6:lengthi5ee" {
		t.Fatalf("raw info = %q", raw["info"])
	}
	if n, _ := Int(m, "zzz"); n != 1 {
		t.Fatalf("zzz = %v", m["zzz"])
	}
}

func TestDecodePrefixLeavesTrailer(t *testing.T) {
	v, rest, err := DecodePrefix([]byte("d8:msg_typei1e5:piecei0eeRAWBYTES"))
	if err != nil || string(rest) != "RAWBYTES" {
		t.Fatalf("%v %q %v", v, rest, err)
	}
}

func TestRejectsMalformed(t *testing.T) {
	for _, in := range []string{
		"", "i", "ie", "i-e", "i-0e", "i03e", "i1", "l", "d", "di1ei2ee",
		"3:ab", "-1:a", "01:a", "x", "i1ei2e", "99999999999:a",
		"i99999999999999999999e", strings.Repeat("l", MaxDepth+5) + strings.Repeat("e", MaxDepth+5),
	} {
		if _, err := Decode([]byte(in)); !errors.Is(err, ErrSyntax) {
			t.Errorf("%q: %v", in, err)
		}
	}
}

func TestEncodeRejectsUnknownTypes(t *testing.T) {
	if _, err := Encode(3.5); err == nil {
		t.Fatal("float must not encode")
	}
}

func FuzzDecode(f *testing.F) {
	for _, s := range []string{"d4:infod4:name3:abcee", "li1ei-2e3:abce", "i0e", "0:", "d1:ad1:bleee"} {
		f.Add([]byte(s))
	}
	f.Fuzz(func(t *testing.T, data []byte) {
		v, err := Decode(data)
		if err != nil {
			return
		}
		raw, err := Encode(v)
		if err != nil {
			t.Fatalf("decoded value does not encode: %v", err)
		}
		v2, err := Decode(raw)
		if err != nil || !reflect.DeepEqual(v, v2) {
			t.Fatalf("re-decode mismatch: %v", err)
		}
	})
}
