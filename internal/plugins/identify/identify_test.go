package identify

import (
	"testing"

	"github.com/enrell/lain/internal/contracts"
)

func TestAnimeReleaseNames(t *testing.T) {
	cases := []struct {
		file    string
		title   string
		season  int
		episode int
		minConf float64
	}{
		{"[Fansub-A] Frieren - 12 [1080p][HEVC x265 10bit][Multi-Subs].mkv", "Frieren", 0, 12, 0.8},
		{"[Fansub-B] Solo Leveling S02E05 (1080p) [7A3B2C1D].mkv", "Solo Leveling", 2, 5, 0.8},
		{"[Fansub-C] One Piece - 1090 [1080p][HEVC x265 10bit][Dual-Audio].mkv", "One Piece", 0, 1090, 0.8},
	}
	for _, tc := range cases {
		p, ok := IdentifyAnime(contracts.Candidate{Path: "/lib/" + tc.file, LibraryID: "lib-1"})
		if !ok {
			t.Fatalf("%s: declined, want accept", tc.file)
		}
		if p.Title != tc.title {
			t.Errorf("%s: title %q, want %q", tc.file, p.Title, tc.title)
		}
		if p.Season != tc.season || p.Episode != tc.episode {
			t.Errorf("%s: s/e %d/%d, want %d/%d", tc.file, p.Season, p.Episode, tc.season, tc.episode)
		}
		if p.Confidence < tc.minConf {
			t.Errorf("%s: confidence %f < %f", tc.file, p.Confidence, tc.minConf)
		}
		if p.PluginID != AnimeID {
			t.Errorf("%s: plugin %q, want %q", tc.file, p.PluginID, AnimeID)
		}
	}
}

func TestAnimeRealLibraryPatterns(t *testing.T) {
	// Structures observed in real collections; group tags are fictional
	// placeholders per AGENTS.md naming hygiene.
	cases := []struct {
		file    string
		title   string
		season  int
		episode int
		year    int
		accept  bool
	}{
		// [Fansub-A]-style: dash episode, END marker, season words.
		{"[Fansub-A] Darling in the FranXX - 13 [1080p][Multiple Subtitle].mkv", "Darling in the FranXX", 0, 13, 0, true},
		{"[Fansub-A] Darling in the FranXX - 09 [1080p][Multiple Subtitle].mkv", "Darling in the FranXX", 0, 9, 0, true},
		{"[Fansub-A] Darling in the FranXX - 24 END [1080p][Multiple Subtitle].mkv", "Darling in the FranXX", 0, 24, 0, true},
		{"[Fansub-A] Tensei Shitara Slime Datta Ken 4th Season - 20 [1080p WEBRip HEVC AAC][MultiSub][B19A6FE6].mkv", "Tensei Shitara Slime Datta Ken", 4, 20, 0, true},
		{"[Fansub-A] Mairimashita Iruma-kun 4th Season - 21 [1080p WEB-DL AVC AAC][MultiSub][EECE1DAB].mkv", "Mairimashita Iruma kun", 4, 21, 0, true},
		{"[Fansub-A] Mushoku Tensei III - Isekai Ittara Honki Dasu - 11 [1080p WEB-DL AVC AAC][MultiSub][1E63CFD7].mkv", "Mushoku Tensei III Isekai Ittara Honki Dasu", 0, 11, 0, true},
		// [Fansub-B]-style: SxxExx with dashes, S00 specials.
		{"[Fansub-D] The Future Diary - S01E16 - 1080p BluRay AV1 Opus 2.0 Dual Audio.mkv", "The Future Diary", 1, 16, 0, true},
		{"[Fansub-D] The Future Diary - S00E02 - Redial - 1080p BluRay AV1 Opus 2.0 Dual Audio.mkv", "The Future Diary", 0, 2, 0, true},
		// Scene dots: year + SxxExx, dual audio.
		{"Lucky.2026.S01E02.1080p.WEB-DL.DUAL.5.1.mkv", "Lucky", 1, 2, 2026, true},
		{"Silo.S01E03.1080p.WEB-DL.DUAL.5.1.mkv", "Silo", 1, 3, 0, true},
		// Double-suffixed scene name.
		{"Silo.S01E01.1080p.WEB-DL.mkv.mp4", "Silo", 1, 1, 0, true},
		// Unicode + dots + year (trailing scene signature survives:
		// metadata slice resolves it).
		{"\u30a2\u30ad\u30e9.Akira.1988.REMASTERED.BluRay.1080p.HDR.HEVC.10bit.FLAC.GRPF.mkv", "アキラ Akira GRPF", 0, 0, 1988, true},
		{"Ghost.in.the.Shell.2017.1080p.BluRay.AV1.Opus.Multi4-Fansub-C.mkv", "Ghost in the Shell Fansub C", 0, 0, 2017, true},
		// Year in parens is metadata, not episode 1995.
		{"1a. Ghost in the Shell - The Movie (1995 - 1080p DUAL Audio).mkv", "1a Ghost in the Shell The Movie", 0, 0, 1995, true},
	}
	for _, tc := range cases {
		p, ok := IdentifyAnime(contracts.Candidate{Path: "/lib/" + tc.file, LibraryID: "lib-1"})
		if ok != tc.accept {
			t.Errorf("%s: accept=%v, want %v (proposal %+v)", tc.file, ok, tc.accept, p)
			continue
		}
		if !tc.accept {
			continue
		}
		if p.Title != tc.title {
			t.Errorf("%s: title %q, want %q", tc.file, p.Title, tc.title)
		}
		if p.Season != tc.season || p.Episode != tc.episode {
			t.Errorf("%s: s/e %d/%d, want %d/%d", tc.file, p.Season, p.Episode, tc.season, tc.episode)
		}
		if p.Year != tc.year {
			t.Errorf("%s: year %d, want %d", tc.file, p.Year, tc.year)
		}
	}
}

func TestAnimeDeclinesPlainNames(t *testing.T) {
	for _, f := range []string{"movie.mp4", "episode 3.mkv", "Lecture 12.mp4"} {
		if _, ok := IdentifyAnime(contracts.Candidate{Path: "/lib/" + f}); ok {
			t.Errorf("%s: accepted, want decline to generic", f)
		}
	}
}

func TestGenericAlwaysAccepts(t *testing.T) {
	for _, f := range []string{"movie.mp4", "track01.flac", "chapter.cbz", "notes.pdf", "pic.heic"} {
		p := IdentifyGeneric(contracts.Candidate{Path: "/lib/" + f})
		if !p.Accepted() {
			t.Errorf("%s: generic declined", f)
		}
	}
	if got := IdentifyGeneric(contracts.Candidate{Path: "/lib/song.flac"}).Kind; got != "audio" {
		t.Errorf("flac kind %q, want audio", got)
	}
}

// Parser fixture corpus: every real-world shape the hand parser must
// keep covering. The benchmark below measures all of them; the table
// in TestAnimeRealLibraryPatterns pins their behavior.
var benchNames = []string{
	"[Fansub-A] Darling in the FranXX - 13 [1080p][Multiple Subtitle].mkv",
	"[Fansub-A] Darling in the FranXX - 24 END [1080p][Multiple Subtitle].mkv",
	"[Fansub-A] Tensei Shitara Slime Datta Ken 4th Season - 20 [1080p WEBRip HEVC AAC][MultiSub][B19A6FE6].mkv",
	"[Fansub-D] The Future Diary - S01E16 - 1080p BluRay AV1 Opus 2.0 Dual Audio.mkv",
	"[Fansub-D] The Future Diary - S00E02 - Redial - 1080p BluRay AV1 Opus 2.0 Dual Audio.mkv",
	"Lucky.2026.S01E02.1080p.WEB-DL.DUAL.5.1.mkv",
	"Silo.S01E01.1080p.WEB-DL.mkv.mp4",
	"アキラ.Akira.1988.REMASTERED.BluRay.1080p.HDR.HEVC.10bit.FLAC.GRPF.mkv",
	"movie.mp4",
}

func BenchmarkIdentifyAnime(b *testing.B) {
	cands := make([]contracts.Candidate, len(benchNames))
	for i, n := range benchNames {
		cands[i] = contracts.Candidate{Path: "/lib/" + n, LibraryID: "lib-1"}
	}
	b.ResetTimer()
	for i := 0; i < b.N; i++ {
		IdentifyAnime(cands[i%len(cands)])
	}
}
