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
		{"[Erai-raws] Frieren - 12 [1080p][HEVC x265 10bit][Multi-Subs].mkv", "Frieren", 0, 12, 0.8},
		{"[SubsPlease] Solo Leveling S02E05 (1080p) [7A3B2C1D].mkv", "Solo Leveling", 2, 5, 0.8},
		{"[Judas] One Piece - 1090 [1080p][HEVC x265 10bit][Dual-Audio].mkv", "One Piece", 0, 1090, 0.8},
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
