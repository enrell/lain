package subtitle

import (
	"strings"
	"testing"
	"time"
)

func ms(n int) time.Duration { return time.Duration(n) * time.Millisecond }

func TestParseSRT(t *testing.T) {
	in := "\xef\xbb\xbf1\r\n00:00:01,000 --> 00:00:02,500\r\nHello <i>there</i>\r\nsecond line\r\n\r\n" +
		"2\n00:01:02.100 --> 00:01:03,000 X1:10\n<font color=\"#ff0\">Yellow</font>\n\n\n" +
		"3\n00:01:04,000 --> 00:01:05,000\n\n" + // empty text: dropped
		"garbage\n\n" +
		"4\n01:00:00,000 --> 01:00:01,000\nLast\n"
	cues, err := Parse([]byte(in))
	if err != nil {
		t.Fatal(err)
	}
	if len(cues) != 3 {
		t.Fatalf("cues = %+v", cues)
	}
	if cues[0].Start != ms(1000) || cues[0].End != ms(2500) || cues[0].Text != "Hello <i>there</i>\nsecond line" {
		t.Fatalf("cue 0 = %+v", cues[0])
	}
	if cues[1].Start != ms(62100) || cues[1].Text != "Yellow" {
		t.Fatalf("cue 1 = %+v", cues[1])
	}
	if cues[2].Start != time.Hour {
		t.Fatalf("cue 2 = %+v", cues[2])
	}
}

func TestParseVTT(t *testing.T) {
	in := "WEBVTT - title\n\nNOTE a comment\nspanning lines\n\nSTYLE\n::cue { color: red }\n\n" +
		"intro\n00:01.000 --> 00:02.000 align:start line:0\nShort <b>form</b>\n\n" +
		"00:00:03.000 --> 00:00:04.000\n<v Roger>Voice</v>\n"
	cues, err := Parse([]byte(in))
	if err != nil {
		t.Fatal(err)
	}
	if len(cues) != 2 || cues[0].Start != ms(1000) || cues[0].Text != "Short <b>form</b>" || cues[1].Text != "<v Roger>Voice</v>" {
		t.Fatalf("cues = %+v", cues)
	}
}

func TestParseASS(t *testing.T) {
	in := "[Script Info]\nTitle: x\n\n[V4+ Styles]\nFormat: Name, Fontname\nStyle: Default,Arial\n\n" +
		"[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n" +
		"Dialogue: 0,0:00:01.50,0:00:03.00,Default,,0,0,0,,{\\an8}Top, with a comma\\NNext line\n" +
		"Comment: 0,0:00:04.00,0:00:05.00,Default,,0,0,0,,not shown\n" +
		"Dialogue: 0,0:00:06.00,0:00:07.25,Default,,0,0,0,,{\\i1}Italic{\\i0}\\hspace\n" +
		"Dialogue: 0,0:00:08.00,0:00:09.00,Default,,0,0,0,,{\\p1}m 0 0 l 100 0{\\p0}\n"
	cues, err := Parse([]byte(in))
	if err != nil {
		t.Fatal(err)
	}
	if len(cues) != 2 {
		t.Fatalf("cues = %+v", cues)
	}
	if cues[0].Start != ms(1500) || cues[0].Text != "Top, with a comma\nNext line" {
		t.Fatalf("cue 0 = %+v", cues[0])
	}
	if cues[1].End != ms(7250) || cues[1].Text != "Italic space" {
		t.Fatalf("cue 1 = %+v", cues[1])
	}
}

func TestWebVTTOutput(t *testing.T) {
	out := string(WebVTT([]Cue{
		{Start: ms(1000), End: ms(2500), Text: "a --> b\n\nc"},
		{Start: time.Hour + ms(61001), End: time.Hour + ms(62000), Text: "<i>x</i> & y"},
	}))
	want := "WEBVTT\n\n00:00:01.000 --> 00:00:02.500\na --&gt; b\nc\n\n01:01:01.001 --> 01:01:02.000\n<i>x</i> &amp; y\n\n"
	if out != want {
		t.Fatalf("got\n%q\nwant\n%q", out, want)
	}
}

func TestDecodeText(t *testing.T) {
	cases := map[string]string{
		"\xef\xbb\xbfcaf\xc3\xa9":              "café",
		"caf\xe9 \x93quoted\x94":               "café “quoted”", // Windows-1252
		"\xff\xfeh\x00i\x00":                   "hi",            // UTF-16LE with BOM
		"\xfe\xff\x00h\x00i":                   "hi",            // UTF-16BE with BOM
		"plain ascii":                          "plain ascii",
		"\xe3\x81\x93\xe3\x82\x93\xe3\x81\xab": "こんに",
	}
	for in, want := range cases {
		if got := string(DecodeText([]byte(in))); got != want {
			t.Errorf("%q -> %q, want %q", in, got, want)
		}
	}
}

func TestStats(t *testing.T) {
	st := Measure([]Cue{{Start: ms(5000), End: ms(6000)}, {Start: ms(1000), End: ms(2000)}, {Start: ms(9000), End: ms(9500)}})
	if st.Cues != 3 || st.First != ms(1000) || st.Last != ms(9500) {
		t.Fatalf("stats = %+v", st)
	}
}

func TestRejectsHostileInput(t *testing.T) {
	if _, err := Parse([]byte(strings.Repeat("x", MaxBytes+1))); err == nil {
		t.Fatal("oversized input must be refused")
	}
	if _, err := Parse([]byte("nothing here\n")); err == nil {
		t.Fatal("a file without cues is an error")
	}
	// End before start is fixed up, not trusted.
	cues, err := Parse([]byte("1\n00:00:05,000 --> 00:00:01,000\nx\n"))
	if err != nil || cues[0].End < cues[0].Start {
		t.Fatalf("%+v %v", cues, err)
	}
}

func FuzzParse(f *testing.F) {
	f.Add([]byte("1\n00:00:01,000 --> 00:00:02,000\nhi\n"))
	f.Add([]byte("WEBVTT\n\n00:01.000 --> 00:02.000\nhi\n"))
	f.Add([]byte("[Events]\nFormat: Start, End, Text\nDialogue: 0:00:01.00,0:00:02.00,hi\n"))
	f.Fuzz(func(t *testing.T, data []byte) {
		cues, err := Parse(data)
		if err != nil {
			return
		}
		if len(cues) > MaxCues {
			t.Fatal("unbounded cues")
		}
		for _, c := range cues {
			if c.End < c.Start || c.Start < 0 {
				t.Fatalf("bad timing %+v", c)
			}
		}
		out := WebVTT(cues)
		if back, err := Parse(out); err != nil || len(back) != len(cues) {
			t.Fatalf("round trip: %d cues -> %d (%v)", len(cues), len(back), err)
		}
	})
}
