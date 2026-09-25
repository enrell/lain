package gateway

import (
	"net/http"
	"os"

	"github.com/enrell/lain/internal/auth"
)

// handleItemDelete removes the item's file from disk (D-073, admin
// only). The catalog row is never hard-deleted: it becomes `missing`
// per D-068, so progress and metadata stay attached and a file that
// returns under the same path restores the entry on the next scan.
// The flag is asserted here rather than left to the watcher because
// LAIN_WATCH=0 installs must converge too. Deleting an already-missing
// item is a no-op that still answers 200 — the end state is identical.
func (s *Server) handleItemDelete(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	id := r.PathValue("id")
	it, ok := s.cat.Get(id)
	if !ok {
		writeErr(w, 404, "unknown item")
		return
	}
	if err := os.Remove(it.FilePath); err != nil && !os.IsNotExist(err) {
		s.logger().Error("item file delete failed", "req", reqIDOf(r), "item", id, "path", it.FilePath, "err", err.Error())
		writeErr(w, 500, err.Error())
		return
	}
	if _, err := s.cat.SetMissing(id, true); err != nil {
		s.logger().Error("item missing-flag failed after file delete", "req", reqIDOf(r), "item", id, "err", err.Error())
		writeErr(w, 500, "file removed, but the catalog entry could not be marked missing")
		return
	}
	it.Missing = true
	writeJSON(w, 200, it)
}
