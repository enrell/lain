package ingest


import (
	"fmt"
	"math/rand"
	"os"
	"path/filepath"
	"sort"
	"testing"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/core"
	"github.com/enrell/lain/internal/kv"
	"github.com/enrell/lain/internal/plugins/catalog"
	"github.com/enrell/lain/internal/plugins/identify"
	"github.com/enrell/lain/internal/plugins/source"
	"github.com/enrell/lain/internal/plugins/userstate"
)

// Model-based test: a seeded random walk over filesystem mutations —
// create, delete, rename, restore, mkdir, permission failure — each
// followed by a real scan. After every step the catalog must equal the
// model: live paths are present and not missing, deleted paths stay as
// missing rows, a same-path restore clears the flag, a failed walk
// marks nothing, and progress rows are never swept. A fresh-catalog
// scan of the same tree acts as the differential oracle for the live
// set.
//
// Deterministic under a fixed seed; bump it to explore other walks.
func TestModelScanReconcile(t *testing.T) {
	if os.Geteuid() == 0 {
		t.Skip("running as root: chmod 000 would not break the walk")
	}
	// Several seeds = several walks; a failure names its seed for replay.
	for _, seed := range []int64{20260923, 7, 424242, 987654321} {
		t.Run(fmt.Sprintf("seed-%d", seed), func(t *testing.T) {
			modelWalk(t, seed)
		})
	}
}

func modelWalk(t *testing.T, seed int64) {
	db, err := kv.Open(t.TempDir())
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { db.Close() })
	cat, err := catalog.New(db)
	if err != nil {
		t.Fatal(err)
	}
	us, err := userstate.New(db)
	if err != nil {
		t.Fatal(err)
	}
	reg := core.NewRegistry(core.DefaultComposition())
	reg.Register(source.Provider{})
	reg.Register(identify.Anime{})
	reg.Register(identify.Generic{})
	reg.Register(cat)
	r := &Runner{Reg: reg, Cat: cat}

	root := t.TempDir()
	lib := contracts.Library{ID: "l1", Type: "anime", Path: root}
	scan := func() contracts.ScanStats {
		t.Helper()
		st, err := r.Run(ScanInput{Libraries: []contracts.Library{lib}})
		if err != nil {
			t.Fatalf("scan: %v", err)
		}
		return st
	}

	rng := rand.New(rand.NewSource(seed))
	live := map[string]string{} // path -> fingerprint
	dead := map[string]string{} // path -> fingerprint (expected missing rows)
	fingerprint := func(path string) string {
		p, _ := identify.IdentifyAnime(contracts.Candidate{Path: path})
		return catalog.Fingerprint("l1", p.Kind, p.Title, p.Season, p.Episode, p.Year)
	}
	progressFor := map[string]bool{} // item ids that must keep progress
	// aliases maps fingerprint -> paths the item previously lived at.
	// ReconcileMoves merges them into Aliases (capped at 8). An old path
	// may legitimately become the live FilePath again (A->B->A), so the
	// invariant is membership in Aliases-or-FilePath, not row absence.
	aliases := map[string]map[string]bool{}
	ep := 0
	name := func() string {
		ep++
		return fmt.Sprintf("Show - S01E%02d [1080p].mkv", ep)
	}
	sub := filepath.Join(root, "sub")
	if err := os.MkdirAll(sub, 0o755); err != nil {
		t.Fatal(err)
	}

	livePaths := func() []string {
		out := make([]string, 0, len(live))
		for p := range live {
			out = append(out, p)
		}
		sort.Strings(out)
		return out
	}
	missingNow := func() []string {
		var out []string
		for _, it := range cat.List() {
			if it.LibraryID == "l1" && it.Missing {
				out = append(out, it.FilePath)
			}
		}
		sort.Strings(out)
		return out
	}

	putProgress := func() {
		for _, it := range cat.List() {
			if it.LibraryID == "l1" && !it.Missing && !progressFor[it.ID] && rng.Intn(2) == 0 {
				if _, err := us.Put(userstate.PutInput{UserID: "u", Progress: contracts.Progress{
					ItemID: it.ID, PositionSec: 42,
				}}); err != nil {
					t.Fatalf("put progress: %v", err)
				}
				progressFor[it.ID] = true
			}
		}
	}

	checkProgress := func() {
		t.Helper()
		for id := range progressFor {
			p, ok := us.Get("u", id)
			if !ok || p.ItemID != id {
				t.Fatalf("progress for %s lost: %+v", id, p)
			}
		}
	}

	checkState := func(step int) {
		t.Helper()
		items := map[string]contracts.CatalogItem{}
		for _, it := range cat.List() {
			if it.LibraryID == "l1" {
				items[it.FilePath] = it
			}
		}
		liveFp := map[string]bool{}
		for p, fp := range live {
			liveFp[fp] = true
			it, ok := items[p]
			if !ok {
				t.Fatalf("step %d: live file %s has no catalog row", step, p)
			}
			if it.Missing {
				t.Fatalf("step %d: live file %s is marked missing", step, p)
			}
		}
		for p, fp := range dead {
			it, ok := items[p]
			if ok {
				if !it.Missing {
					t.Fatalf("step %d: dead file %s not marked missing", step, p)
				}
			} else if !liveFp[fp] {
				// A missing row may legitimately disappear only when its
				// fingerprint was reconciled onto a live path.
				t.Fatalf("step %d: dead file %s vanished from catalog entirely", step, p)
			}
		}
		for p := range items {
			if _, ok := live[p]; !ok {
				if _, ok := dead[p]; !ok {
					t.Fatalf("step %d: phantom catalog row for %s", step, p)
				}
			}
		}
		// Reconcile bookkeeping: every path a live item previously
		// occupied is either its alias or its current FilePath (move-back).
		byFp := map[string]contracts.CatalogItem{}
		for _, it := range items {
			byFp[fingerprint(it.FilePath)] = it
		}
		for fp, seen := range aliases {
			it, ok := byFp[fp]
			if !ok || len(it.Aliases) >= 8 {
				continue // row gone or alias cap evicted history
			}
			for a := range seen {
				if a == it.FilePath {
					continue
				}
				found := false
				for _, x := range it.Aliases {
					if x == a {
						found = true
					}
				}
				if !found {
					t.Fatalf("step %d: item at %s lost alias %s (aliases %v)", step, it.FilePath, a, it.Aliases)
				}
			}
		}
		// A dead row whose fingerprint was adopted by a live item has no
		// missing row anymore — subtract it from the expected count.
		adopted := 0
		for _, fp := range dead {
			if liveFp[fp] {
				adopted++
			}
		}
		if len(items) != len(live)+len(dead)-adopted {
			t.Fatalf("step %d: catalog has %d rows, model expects %d live + %d dead - %d adopted",
				step, len(items), len(live), len(dead), adopted)
		}
		checkProgress()
	}

	// differential: a fresh catalog over the same tree must agree on the
	// live set — the incremental path and the clean path converge.
	freshLiveSet := func() map[string]bool {
		t.Helper()
		db2, err := kv.Open(t.TempDir())
		if err != nil {
			t.Fatal(err)
		}
		defer db2.Close()
		cat2, err := catalog.New(db2)
		if err != nil {
			t.Fatal(err)
		}
		reg2 := core.NewRegistry(core.DefaultComposition())
		reg2.Register(source.Provider{})
		reg2.Register(identify.Anime{})
		reg2.Register(identify.Generic{})
		reg2.Register(cat2)
		r2 := &Runner{Reg: reg2, Cat: cat2}
		if _, err := r2.Run(ScanInput{Libraries: []contracts.Library{lib}}); err != nil {
			t.Fatalf("oracle scan: %v", err)
		}
		out := map[string]bool{}
		for _, it := range cat2.List() {
			out[it.FilePath] = true
		}
		return out
	}

	for step := 0; step < 200; step++ {
		paths := livePaths()
		op := rng.Intn(100)
		brokeWalk := false
		switch {
		case op < 35: // create
			dir := root
			if rng.Intn(3) == 0 {
				dir = sub
			}
			p := filepath.Join(dir, name())
			writeFile(t, p)
			live[p] = fingerprint(p)
		case op < 55 && len(paths) > 0: // delete
			p := paths[rng.Intn(len(paths))]
			if err := os.Remove(p); err != nil {
				t.Fatal(err)
			}
			dead[p] = live[p]
			delete(live, p)
		case op < 70 && len(paths) > 0: // rename
			p := paths[rng.Intn(len(paths))]
			base := filepath.Base(p)
			var dst string
			if rng.Intn(2) == 0 {
				// Move across dirs keeping the basename: the fingerprint
				// survives, so the scan must reconcile the row onto the
				// new path — same id, old path becomes an alias, no
				// missing row for the old path.
				if filepath.Dir(p) == root {
					dst = filepath.Join(sub, base)
				} else {
					dst = filepath.Join(root, base)
				}
				if err := os.Rename(p, dst); err != nil {
					t.Fatal(err)
				}
				fp := live[p]
				// The destination may be a tombstone: the file is back,
				// so its missing row resurrects — and because the row's
				// FilePath already equals dst, reconcile records no
				// alias for the vacated source path.
				_, wasDead := dead[dst]
				delete(dead, dst)
				live[dst] = fp
				delete(live, p)
				if !wasDead {
					if aliases[fp] == nil {
						aliases[fp] = map[string]bool{}
					}
					aliases[fp][p] = true
				}
			} else {
				// Rename the basename: the parsed title changes, so this
				// is honestly delete+create — old path goes missing.
				if filepath.Dir(p) == root {
					dst = filepath.Join(sub, "moved-"+base)
				} else {
					dst = filepath.Join(root, "moved-"+base)
				}
				if err := os.Rename(p, dst); err != nil {
					t.Fatal(err)
				}
				dead[p] = live[p]
				delete(live, p)
				live[dst] = fingerprint(dst)
				delete(dead, dst) // resurrected tombstone, if any
			}
		case op < 80 && len(dead) > 0: // restore a deleted path
			var pick string
			for p := range dead {
				pick = p
				break
			}
			writeFile(t, pick)
			live[pick] = dead[pick]
			delete(dead, pick)
		case op < 85: // new empty dir
			if err := os.MkdirAll(filepath.Join(root, fmt.Sprintf("d%d", step)), 0o755); err != nil {
				t.Fatal(err)
			}
		case op < 93: // break the walk: root unreadable for one scan
			if err := os.Chmod(root, 0o000); err != nil {
				t.Fatal(err)
			}
			brokeWalk = true
		default: // pure rescan
		}

		before := missingNow()
		stats := scan()
		if err := os.Chmod(root, 0o755); err != nil {
			t.Fatal(err)
		}

		if brokeWalk {
			if stats.WalkErrors == 0 {
				t.Fatalf("step %d: unreadable root produced no walk error", step)
			}
			after := missingNow()
			if fmt.Sprint(before) != fmt.Sprint(after) {
				t.Fatalf("step %d: failed walk changed the missing set: %v -> %v", step, before, after)
			}
			checkProgress()
			continue
		}

		checkState(step)

		// Differential oracle every 25 steps — cheap enough at this size.
		if step%25 == 0 {
			fresh := freshLiveSet()
			if len(fresh) != len(live) {
				t.Fatalf("step %d: fresh scan sees %d files, model has %d", step, len(fresh), len(live))
			}
			for p := range live {
				if !fresh[p] {
					t.Fatalf("step %d: fresh scan missing %s", step, p)
				}
			}
		}
		putProgress()
	}
	t.Logf("model walk finished: %d live, %d missing rows, %d progress rows",
		len(live), len(dead), len(progressFor))
}

// TestMoveBackPreservesAliases is the distilled regression for the bug
// the model walk found: moving a file A->B->A made its path-derived ID
// collide with its own row, skipping reconcile and wiping Aliases.
func TestMoveBackPreservesAliases(t *testing.T) {
	r, cat := testRunner(t)
	root := t.TempDir()
	sub := filepath.Join(root, "sub")
	if err := os.MkdirAll(sub, 0o755); err != nil {
		t.Fatal(err)
	}
	lib := contracts.Library{ID: "l1", Type: "anime", Path: root}
	scan := func() {
		if _, err := r.Run(ScanInput{Libraries: []contracts.Library{lib}}); err != nil {
			t.Fatal(err)
		}
	}
	a := filepath.Join(root, "Show - S01E01 [1080p].mkv")
	b := filepath.Join(sub, "Show - S01E01 [1080p].mkv")
	writeFile(t, a)
	scan()
	for _, mv := range [][2]string{{a, b}, {b, a}, {a, b}} {
		if err := os.Rename(mv[0], mv[1]); err != nil {
			t.Fatal(err)
		}
		scan()
	}
	items := cat.List()
	if len(items) != 1 {
		t.Fatalf("want 1 row after move-back cycles, got %d", len(items))
	}
	it := items[0]
	if it.FilePath != b {
		t.Fatalf("FilePath = %s, want %s", it.FilePath, b)
	}
	if !contains(it.Aliases, a) || !contains(it.Aliases, b) {
		t.Fatalf("aliases %v lost move history (want %s and %s)", it.Aliases, a, b)
	}
}

func contains(xs []string, v string) bool {
	for _, x := range xs {
		if x == v {
			return true
		}
	}
	return false
}
