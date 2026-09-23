package main


import "testing"

func TestParseByteSize(t *testing.T) {
	for raw, want := range map[string]int64{
		"1":      1,
		"512MiB": 512 << 20,
		"20GiB":  20 << 30,
		"2TiB":   2 << 40,
	} {
		got, err := parseByteSize(raw)
		if err != nil || got != want {
			t.Fatalf("parseByteSize(%q)=(%d,%v), want (%d,nil)", raw, got, err, want)
		}
	}
	for _, raw := range []string{"", "0", "-1GiB", "1GB", "lots"} {
		if _, err := parseByteSize(raw); err == nil {
			t.Fatalf("parseByteSize(%q) must fail", raw)
		}
	}
}
