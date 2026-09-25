// Theme endpoint: the palette comes from the lain.ui.theme@1
// capability (D-076); the gateway relays it without any derivation of
// its own. An unbound or failing provider answers 503 — the frontend
// keeps its compiled-in CSS defaults either way.
package gateway

import (
	"net/http"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/plugins/theme"
)

func (s *Server) handleTheme(w http.ResponseWriter, _ *http.Request) {
	w.Header().Set("Cache-Control", "no-store")
	out, _, err := s.reg.CallOne(contracts.CapUITheme, theme.Input{ColorsPath: s.themePath})
	if err != nil {
		writeErr(w, http.StatusServiceUnavailable, err.Error())
		return
	}
	palette, ok := out.(theme.Palette)
	if !ok {
		writeErr(w, http.StatusServiceUnavailable, "theme provider returned a bad shape")
		return
	}
	writeJSON(w, http.StatusOK, palette)
}
