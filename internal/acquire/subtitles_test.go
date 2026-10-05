package acquire

import (
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/kv"
)

// subWorld fakes the subtitle provider and the probe; files are served
// by a local HTTP server (A-13: no network).
type subWorld struct {
	files    *httptest.Server
	bodies   map[string]string // file id -> subtitle body
	cands    []contracts.SubtitleCandidate
	searches []contracts.SubtitleSearchInput
	info     contracts.MediaInfo
}

func newSubWorld(t *testing.T) *subWorld {
	sw := &subWorld{bodies: map[string]string{}}
	sw.files = httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		body, ok := sw.bodies[strings.TrimPrefix(r.URL.Path, "/f/")]
		if !ok {
			http.NotFound(w, r)
			return
		}
		_, _ = w.Write([]byte(body))
	}))
	t.Cleanup(sw.files.Close)
	return sw
}

func (sw *subWorld) wire(m *Manager) {
	m.d.Probe = func(string) (contracts.MediaInfo, error) { return sw.info, nil }
	m.d.SubtitleSearch = func(in contracts.SubtitleSearchInput) ([]contracts.SubtitleCandidate, error) {
		sw.searches = append(sw.searches, in)
		return sw.cands, nil
	}
	m.d.SubtitleDownload = func(in contracts.SubtitleDownloadInput) (contracts.SubtitleDownloadOutput, error) {
		return contracts.SubtitleDownloadOutput{Link: sw.files.URL + "/f/" + in.FileID, Remaining: 9}, nil
	}
}

const goodSRT = "1\n00:00:01,000 --> 00:00:02,000\nOl\xe1\n\n2\n00:20:00,000 --> 00:20:01,000\nfim\n"

func subProfile(t *testing.T, m *Manager, langs ...string) Profile {
	t.Helper()
	p := DefaultProfile()
	p.SubtitleLanguages = langs
	p, err := m.UpdateProfile("default", p)
	if err != nil {
		t.Fatal(err)
	}
	return p
}

func addSubProvider(t *testing.T, m *Manager) contracts.SubtitleProvider {
	t.Helper()
	name, kind, key := "os", contracts.SubtitleOpenSubtitles, "k"
	p, err := m.CreateSubtitleProvider(SubtitleProviderInput{Name: &name, Kind: &kind, APIKey: &key})
	if err != nil {
		t.Fatal(err)
	}
	return p
}

func media(t *testing.T, dir, name string) string {
	t.Helper()
	p := filepath.Join(dir, "SHOW", name)
	_ = os.MkdirAll(filepath.Dir(p), 0o755)
	if err := os.WriteFile(p, make([]byte, 200_000), 0o644); err != nil {
		t.Fatal(err)
	}
	return p
}

func TestWantedSubtitleLanguages(t *testing.T) {
	w := newWorld(t)
	m := w.manager(t, t.TempDir(), settings(0))
	sw := newSubWorld(t)
	sw.wire(m)
	p := subProfile(t, m, "eng", "por", "spa")
	file := media(t, w.lib.Path, "[Fansub-A] SHOW - 01.mkv")
	sw.info = contracts.MediaInfo{Duration: 1440, Streams: []contracts.MediaStream{
		{Index: 1, Type: "audio", Language: "spa"}, {Index: 2, Type: "subtitle", Language: "eng"},
	}}
	_ = os.WriteFile(filepath.Join(w.lib.Path, "SHOW", "[Fansub-A] SHOW - 01.pt-BR.forced.srt"), []byte(goodSRT), 0o644)
	got := m.MissingSubtitles(file, p)
	// eng: embedded; spa: the audio (skip-if-audio); por: only a forced sidecar.
	if strings.Join(got, ",") != "por" {
		t.Fatalf("missing = %v", got)
	}
	_ = os.WriteFile(filepath.Join(w.lib.Path, "SHOW", "[Fansub-A] SHOW - 01.pt.srt"), []byte(goodSRT), 0o644)
	if got := m.MissingSubtitles(file, p); len(got) != 0 {
		t.Fatalf("a full sidecar satisfies the language: %v", got)
	}
	p.SubtitleEvenWithAudio = true
	if got := m.MissingSubtitles(file, p); strings.Join(got, ",") != "spa" {
		t.Fatalf("even with audio: %v", got)
	}
}

func TestSubtitleRanking(t *testing.T) {
	w := newWorld(t)
	m := w.manager(t, t.TempDir(), settings(0))
	sw := newSubWorld(t)
	sw.wire(m)
	addSubProvider(t, m)
	p := subProfile(t, m, "por")
	p.SubtitleHI = HIExclude
	file := media(t, w.lib.Path, "[Fansub-A] SHOW - 05 [1080p].mkv")
	sw.cands = []contracts.SubtitleCandidate{
		{FileID: "1", Language: "por", Downloads: 5000, Release: "[Fansub-B] Show - 05 [720p]"},
		{FileID: "2", Language: "por", HashMatch: true, Downloads: 1},
		{FileID: "3", Language: "por", Release: "[Fansub-A] Show - 05 [1080p]", Downloads: 10},
		{FileID: "4", Language: "por", Forced: true, HashMatch: true},
		{FileID: "5", Language: "por", HI: true, HashMatch: true},
		{FileID: "6", Language: "por", Episode: 6, HashMatch: true},
		{FileID: "7", Language: "eng", HashMatch: true},
	}
	res, err := m.SearchSubtitles(SubtitleTarget{Path: file, Kind: "anime", Title: "Show", Episode: 5}, p, []string{"por"})
	if err != nil {
		t.Fatal(err)
	}
	var order, rejected []string
	for _, c := range res {
		if c.Accepted {
			order = append(order, c.FileID)
		} else {
			rejected = append(rejected, c.FileID)
		}
	}
	if strings.Join(order, ",") != "2,3,1" || strings.Join(rejected, ",") != "4,5,6,7" {
		t.Fatalf("order %v rejected %v (%+v)", order, rejected, res)
	}
	if len(sw.searches) != 1 || sw.searches[0].Hash == "" || sw.searches[0].Episode != 5 || sw.searches[0].Languages[0] != "por" {
		t.Fatalf("search input: %+v", sw.searches)
	}
}

func TestDownloadPlacesASidecarWithoutOverwriting(t *testing.T) {
	w := newWorld(t)
	m := w.manager(t, t.TempDir(), settings(0))
	sw := newSubWorld(t)
	sw.wire(m)
	prov := addSubProvider(t, m)
	file := media(t, w.lib.Path, "[Fansub-A] SHOW - 01.mkv")
	sw.info = contracts.MediaInfo{Duration: 1440}
	sw.bodies["9"] = goodSRT
	c := contracts.SubtitleCandidate{ProviderID: prov.ID, FileID: "9", Language: "por", Region: "pt-BR", FileName: "x.srt"}
	rec, err := m.DownloadSubtitle(file, c, false)
	if err != nil {
		t.Fatal(err)
	}
	want := filepath.Join(w.lib.Path, "SHOW", "[Fansub-A] SHOW - 01.pt-BR.srt")
	if rec.Path != want {
		t.Fatalf("path %s", rec.Path)
	}
	b, _ := os.ReadFile(want)
	if !strings.Contains(string(b), "Olá") {
		t.Fatalf("sidecar must be UTF-8: %q", b)
	}
	if got := m.SubtitleLedger(); len(got) != 1 || got[0].FileID != "9" {
		t.Fatalf("ledger: %+v", got)
	}
	// Same name again: refused unless replacing; replacing holds the old.
	if _, err := m.DownloadSubtitle(file, c, false); CodeOf(err) != CodeState {
		t.Fatalf("second download: %v", err)
	}
	sw.bodies["9"] = strings.Replace(goodSRT, "fim", "novo", 1)
	if _, err := m.DownloadSubtitle(file, c, true); err != nil {
		t.Fatal(err)
	}
	if b, _ := os.ReadFile(want); !strings.Contains(string(b), "novo") {
		t.Fatal("replace did not write the new file")
	}
	held := m.Replaced()
	if len(held) != 1 {
		t.Fatalf("the old sidecar must be held, not deleted: %+v", held)
	}
	// Never next to a file outside every library root.
	outside := filepath.Join(t.TempDir(), "x.mkv")
	_ = os.WriteFile(outside, []byte("v"), 0o644)
	if _, err := m.DownloadSubtitle(outside, c, false); CodeOf(err) != CodeInvalid {
		t.Fatalf("outside the library: %v", err)
	}
}

func TestSyncCheckRefusesMismatchedSubtitles(t *testing.T) {
	w := newWorld(t)
	m := w.manager(t, t.TempDir(), settings(0))
	sw := newSubWorld(t)
	sw.wire(m)
	prov := addSubProvider(t, m)
	file := media(t, w.lib.Path, "[Fansub-A] SHOW - 01.mkv")
	sw.info = contracts.MediaInfo{Duration: 600} // 10 min; the subtitle runs 20 min
	sw.bodies["9"] = goodSRT
	c := contracts.SubtitleCandidate{ProviderID: prov.ID, FileID: "9", Language: "eng"}
	if _, err := m.DownloadSubtitle(file, c, false); CodeOf(err) != CodeSubtitleMismatch {
		t.Fatalf("mismatch: %v", err)
	}
	if _, err := os.Stat(filepath.Join(w.lib.Path, "SHOW", "[Fansub-A] SHOW - 01.en.srt")); !os.IsNotExist(err) {
		t.Fatal("a refused subtitle must not be written")
	}
	p := subProfile(t, m, "eng")
	sw.cands = []contracts.SubtitleCandidate{{ProviderID: prov.ID, FileID: "9", Language: "eng", HashMatch: true}}
	res, _ := m.SearchSubtitles(SubtitleTarget{Path: file, Kind: "anime", Title: "Show", Episode: 1}, p, []string{"eng"})
	if len(res) != 1 || res[0].Accepted {
		t.Fatalf("a refused file must stay refused: %+v", res)
	}
	// Unknown duration (probe failed): the check cannot run, the file is kept.
	sw.info = contracts.MediaInfo{}
	sw.bodies["10"] = goodSRT
	if _, err := m.DownloadSubtitle(file, contracts.SubtitleCandidate{ProviderID: prov.ID, FileID: "10", Language: "eng"}, false); err != nil {
		t.Fatal(err)
	}
	// A file with no cues is refused too.
	sw.bodies["11"] = "nothing"
	if _, err := m.DownloadSubtitle(file, contracts.SubtitleCandidate{ProviderID: prov.ID, FileID: "11", Language: "spa"}, false); CodeOf(err) != CodeSubtitleMismatch {
		t.Fatalf("empty subtitle: %v", err)
	}
}

func TestSubtitlesFollowAMonitoredImport(t *testing.T) {
	w := newWorld(t)
	m := w.manager(t, t.TempDir(), settings(0))
	sw := newSubWorld(t)
	sw.wire(m)
	prov := addSubProvider(t, m)
	subProfile(t, m, "eng")
	sw.info = contracts.MediaInfo{Duration: 1440}
	sw.bodies["9"] = goodSRT
	sw.cands = []contracts.SubtitleCandidate{{ProviderID: prov.ID, FileID: "9", Language: "eng", HashMatch: true}}
	addIndexer(t, m, w)
	mon := monitor(t, m, w, Monitored{Title: "Show", From: 1, To: 2})
	rep, _ := m.SearchMonitored(mon.ID)
	if len(rep.Grabbed) != 1 {
		t.Fatalf("search: %+v", rep)
	}
	waitGrab(t, m, rep.Grabbed[0], GrabDone)
	waitFor(t, func() bool {
		_, err := os.Stat(filepath.Join(w.lib.Path, "SHOW", "[Fansub-A] SHOW - 01.en.srt"))
		return err == nil
	})
	st, err := m.SubtitleStatus(mon.ID)
	if err != nil || len(st) != 2 || len(st[0].Missing) != 0 {
		t.Fatalf("status: %+v %v", st, err)
	}
}

func waitFor(t *testing.T, cond func() bool) {
	t.Helper()
	deadline := time.Now().Add(15 * time.Second)
	for time.Now().Before(deadline) {
		if cond() {
			return
		}
		time.Sleep(50 * time.Millisecond)
	}
	t.Fatal("condition never became true")
}

func TestSubtitleRefusalsCanBeListedAndCleared(t *testing.T) {
	w := newWorld(t)
	m := w.manager(t, t.TempDir(), settings(0))
	sw := newSubWorld(t)
	sw.wire(m)
	prov := addSubProvider(t, m)
	file := media(t, w.lib.Path, "[Fansub-A] SHOW - 01.mkv")
	sw.info = contracts.MediaInfo{Duration: 600}
	sw.bodies["9"] = goodSRT // runs 20 min: refused
	sw.bodies["10"] = "nothing"
	for _, id := range []string{"9", "10"} {
		if _, err := m.DownloadSubtitle(file, contracts.SubtitleCandidate{ProviderID: prov.ID, FileID: id, Language: "eng"}, false); CodeOf(err) != CodeSubtitleMismatch {
			t.Fatalf("file %s: %v", id, err)
		}
	}
	refs := m.SubtitleRefusals()
	if len(refs) != 2 {
		t.Fatalf("refusals: %+v", refs)
	}
	for _, r := range refs {
		if r.ProviderID != prov.ID || r.Reason == "" || strings.HasPrefix(r.Reason, CodeSubtitleMismatch) || r.MediaPath != file || r.At == 0 {
			t.Fatalf("refusal record: %+v", r)
		}
	}
	p := subProfile(t, m, "eng")
	sw.cands = []contracts.SubtitleCandidate{{ProviderID: prov.ID, FileID: "9", Language: "eng"}}
	target := SubtitleTarget{Path: file, Kind: "anime", Title: "Show", Episode: 1}
	if res, _ := m.SearchSubtitles(target, p, []string{"eng"}); len(res) != 1 || res[0].Accepted {
		t.Fatalf("refused file must be rejected: %+v", res)
	}
	// Clearing one makes it eligible again; an unknown one is not found.
	if err := m.ClearSubtitleRefusal(prov.ID, "9"); err != nil {
		t.Fatal(err)
	}
	if err := m.ClearSubtitleRefusal(prov.ID, "9"); CodeOf(err) != CodeNotFound {
		t.Fatalf("second clear: %v", err)
	}
	if res, _ := m.SearchSubtitles(target, p, []string{"eng"}); len(res) != 1 || !res[0].Accepted {
		t.Fatalf("cleared file must be eligible: %+v", res)
	}
	if n := m.ClearSubtitleRefusals(); n != 1 || len(m.SubtitleRefusals()) != 0 {
		t.Fatalf("clear all: %d, %+v", n, m.SubtitleRefusals())
	}
}

func TestSubtitleRefusalsReadLegacyValues(t *testing.T) {
	w := newWorld(t)
	m := w.manager(t, t.TempDir(), settings(0))
	// Before refusals carried a record they stored the reason alone.
	if err := putJSON(m.st, kv.BAcqSubBlock, subBlockKey("p1", "42"), "too long"); err != nil {
		t.Fatal(err)
	}
	refs := m.SubtitleRefusals()
	if len(refs) != 1 || refs[0].ProviderID != "p1" || refs[0].FileID != "42" || refs[0].Reason != "too long" {
		t.Fatalf("legacy refusal: %+v", refs)
	}
	if !m.subtitleRefused("p1", "42") {
		t.Fatal("a legacy refusal must still block")
	}
}

func TestRemoveSidecarHoldsItAndNeverDeletes(t *testing.T) {
	w := newWorld(t)
	m := w.manager(t, t.TempDir(), settings(0))
	sw := newSubWorld(t)
	sw.wire(m)
	prov := addSubProvider(t, m)
	file := media(t, w.lib.Path, "[Fansub-A] SHOW - 01.mkv")
	sw.bodies["9"] = goodSRT
	rec, err := m.DownloadSubtitle(file, contracts.SubtitleCandidate{ProviderID: prov.ID, FileID: "9", Language: "eng"}, false)
	if err != nil {
		t.Fatal(err)
	}
	// A sidecar Lain did not write is removable too, the same way.
	own := filepath.Join(filepath.Dir(file), "[Fansub-A] SHOW - 01.ja.ass")
	_ = os.WriteFile(own, []byte("[Script Info]\n"), 0o644)
	other := media(t, w.lib.Path, "[Fansub-A] SHOW - 02.mkv")
	otherSub := filepath.Join(filepath.Dir(other), "[Fansub-A] SHOW - 02.en.srt")
	_ = os.WriteFile(otherSub, []byte(goodSRT), 0o644)

	for _, name := range []string{filepath.Base(rec.Path), filepath.Base(own)} {
		held, err := m.RemoveSidecar(file, name)
		if err != nil {
			t.Fatalf("%s: %v", name, err)
		}
		if _, err := os.Stat(filepath.Join(filepath.Dir(file), name)); !os.IsNotExist(err) {
			t.Fatalf("%s still in the library", name)
		}
		if _, err := os.Stat(held.Path); err != nil || !within(m.holdDir(), held.Path) {
			t.Fatalf("%s must be held, not deleted: %+v %v", name, held, err)
		}
	}
	if len(m.SubtitleLedger()) != 0 {
		t.Fatalf("a removed sidecar leaves the ledger: %+v", m.SubtitleLedger())
	}
	if len(m.Replaced()) != 2 {
		t.Fatalf("held: %+v", m.Replaced())
	}
	// Only sidecars of that media: never another file, never the media,
	// never a path.
	for _, name := range []string{filepath.Base(otherSub), filepath.Base(file), "../SHOW/" + filepath.Base(otherSub), "nope.en.srt"} {
		if _, err := m.RemoveSidecar(file, name); CodeOf(err) != CodeNotFound {
			t.Fatalf("%q: %v", name, err)
		}
	}
	if _, err := os.Stat(otherSub); err != nil {
		t.Fatal("another file's sidecar was touched")
	}
	if _, err := os.Stat(file); err != nil {
		t.Fatal("the media was touched")
	}
}

func TestItemSubtitleStatusUsesTheGoverningProfile(t *testing.T) {
	w := newWorld(t)
	m := w.manager(t, t.TempDir(), settings(0))
	sw := newSubWorld(t)
	sw.wire(m)
	file := media(t, w.lib.Path, "[Fansub-A] SHOW - 01.mkv")
	_ = os.WriteFile(filepath.Join(filepath.Dir(file), "[Fansub-A] SHOW - 01.en.srt"), []byte(goodSRT), 0o644)
	subProfile(t, m, "eng", "por")
	st, langs, err := m.ItemSubtitleStatus(contracts.CatalogItem{ID: "i1", LibraryID: w.lib.ID, Title: "Unmonitored", FilePath: file})
	if err != nil {
		t.Fatal(err)
	}
	if strings.Join(langs, ",") != "eng,por" || strings.Join(st.Missing, ",") != "por" || len(st.Sidecars) != 1 || st.ItemID != "i1" {
		t.Fatalf("status: %+v langs %v", st, langs)
	}
}
