package gateway

import (
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
	"time"
)

// While a scan is running, a dirty drain re-marks the library in the
// provider instead of racing the in-flight scan; once the lock frees,
// one watch scan lands.
func TestWatcherRearmsDuringRunningScan(t *testing.T) {
	srv := testServer(t)
	srv.watchDebounce = 30 * time.Millisecond
	root := t.TempDir()
	lib, err := srv.libs.Create("L", "movie", root)
	if err != nil {
		t.Fatal(err)
	}
	if err := srv.StartWatcher(); err != nil {
		t.Fatal(err)
	}
	// Occupy the scan slot like a manual scan would.
	srv.scanMu.Lock()
	srv.scan = ScanStatus{State: "running", StartedAt: time.Now().Unix(), Trigger: "manual"}
	srv.scanMu.Unlock()
	srv.scanLibrary(lib.ID)
	// Give the pending timer at least one look at the busy state, then
	// free the slot.
	time.Sleep(3 * srv.watchDebounce)
	srv.scanMu.Lock()
	srv.scan = ScanStatus{State: "idle"}
	srv.scanMu.Unlock()
	deadline := time.Now().Add(10 * time.Second)
	for {
		srv.scanMu.Lock()
		st := srv.scan
		srv.scanMu.Unlock()
		if st.State == "done" && st.Trigger == "watch" {
			return
		}
		if st.State == "error" {
			t.Fatalf("watch scan failed: %s", st.Error)
		}
		if time.Now().After(deadline) {
			t.Fatalf("watch scan never landed: %+v", st)
		}
		time.Sleep(20 * time.Millisecond)
	}
}

// The watcher turns a filesystem delete into a missing flag without a
// manual scan (D-068): debounce collapses the event burst, the
// per-library rescan marks the item, and the scan status reports the
// "watch" trigger.
func TestWatcherMarksDeletedFileMissing(t *testing.T) {
	srv := testServer(t)
	srv.watchDebounce = 30 * time.Millisecond
	admin := setupAdmin(t, srv)

	root := t.TempDir()
	gone := filepath.Join(root, "[Fansub-A] Frieren - 12 [1080p].mkv")
	for _, f := range []string{gone, filepath.Join(root, "keep.mkv")} {
		if err := os.WriteFile(f, []byte("fake-media"), 0o644); err != nil {
			t.Fatal(err)
		}
	}
	if rec := do(t, srv, "POST", "/api/libraries", map[string]string{"name": "Anime", "type": "anime", "path": root}, admin); rec.Code != 201 {
		t.Fatalf("library: %d %s", rec.Code, rec.Body.String())
	}
	if rec := do(t, srv, "POST", "/api/library/scan", nil, admin); rec.Code != 202 {
		t.Fatalf("scan: %d", rec.Code)
	}
	waitScan(t, srv, admin)

	if err := srv.StartWatcher(); err != nil {
		t.Fatal(err)
	}
	if err := os.Remove(gone); err != nil {
		t.Fatal(err)
	}

	// Poll the catalog until the watcher-driven scan lands.
	var item struct {
		ID      string `json:"id"`
		Missing bool   `json:"missing"`
	}
	deadline := time.Now().Add(10 * time.Second)
	for {
		rec := do(t, srv, "GET", "/api/catalog", nil, admin)
		var page struct {
			Items []struct {
				ID       string `json:"id"`
				FilePath string `json:"file_path"`
				Missing  bool   `json:"missing"`
			} `json:"items"`
		}
		if err := json.Unmarshal(rec.Body.Bytes(), &page); err != nil {
			t.Fatal(err)
		}
		for _, it := range page.Items {
			if it.FilePath == gone {
				item.ID, item.Missing = it.ID, it.Missing
			}
		}
		if item.Missing {
			break
		}
		if time.Now().After(deadline) {
			t.Fatalf("deleted file never marked missing: %+v", page.Items)
		}
		time.Sleep(50 * time.Millisecond)
	}
	// The row survives: missing is a state, not a deletion.
	if item.ID == "" {
		t.Fatal("missing item must stay in the catalog")
	}
	if rec := do(t, srv, "GET", "/api/items/"+item.ID+"/playback", nil, admin); rec.Code != 200 {
		t.Fatalf("plan: %d", rec.Code)
	}

	// The watch-triggered scan is reported as such, not as a manual one.
	var status struct {
		State   string `json:"state"`
		Trigger string `json:"trigger"`
	}
	rec := do(t, srv, "GET", "/api/library/scan", nil, admin)
	if err := json.Unmarshal(rec.Body.Bytes(), &status); err != nil {
		t.Fatal(err)
	}
	if status.Trigger != "watch" {
		t.Fatalf("trigger=%q, want watch", status.Trigger)
	}
}

// A file deleted between scans answers the playback plan honestly
// instead of failing at stream time.
func TestPlaybackPlanReportsMissingFile(t *testing.T) {
	srv := testServer(t)
	admin := setupAdmin(t, srv)

	root := t.TempDir()
	file := filepath.Join(root, "gone.mkv")
	if err := os.WriteFile(file, []byte("fake-media"), 0o644); err != nil {
		t.Fatal(err)
	}
	if rec := do(t, srv, "POST", "/api/libraries", map[string]string{"name": "L", "type": "movie", "path": root}, admin); rec.Code != 201 {
		t.Fatalf("library: %d %s", rec.Code, rec.Body.String())
	}
	if rec := do(t, srv, "POST", "/api/library/scan", nil, admin); rec.Code != 202 {
		t.Fatalf("scan: %d", rec.Code)
	}
	waitScan(t, srv, admin)

	rec := do(t, srv, "GET", "/api/catalog", nil, admin)
	var page struct {
		Items []struct {
			ID string `json:"id"`
		} `json:"items"`
	}
	if err := json.Unmarshal(rec.Body.Bytes(), &page); err != nil || len(page.Items) != 1 {
		t.Fatalf("catalog: %d %s", rec.Code, rec.Body.String())
	}
	id := page.Items[0].ID

	if err := os.Remove(file); err != nil {
		t.Fatal(err)
	}
	rec = do(t, srv, "GET", "/api/items/"+id+"/playback", nil, admin)
	var plan struct {
		Mode      string `json:"mode"`
		Available bool   `json:"available"`
		Reason    string `json:"reason"`
	}
	if err := json.Unmarshal(rec.Body.Bytes(), &plan); err != nil {
		t.Fatal(err)
	}
	if plan.Available || plan.Reason == "" {
		t.Fatalf("missing file must be unavailable with a reason: %+v", plan)
	}
}
