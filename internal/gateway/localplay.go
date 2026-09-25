package gateway

import (
	"net"
	"net/http"

	"github.com/enrell/lain/internal/auth"
	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/core"
	pluginlocalplay "github.com/enrell/lain/internal/plugins/localplay"
)

// Local playback (D-072): when the browser sits on the same machine as a
// natively running server, the server can spawn mpv/VLC itself — the
// files are already local, so no token or stream URL ever crosses a
// link. Remote browsers get an empty capability and a 403 on play. The
// session itself is the lain.playback.local@1 provider's business
// (D-076): queue, resume, subtitle and player policy all live there.

// localRequest reports whether this request came from the machine the
// server runs on. Forwarded headers mean a proxy hop sits between the
// browser and us, so the loopback peer is the proxy — not proof the
// viewer is local.
func localRequest(r *http.Request) bool {
	if r.Header.Get("Forwarded") != "" || r.Header.Get("X-Forwarded-For") != "" || r.Header.Get("X-Real-IP") != "" {
		return false
	}
	host, _, err := net.SplitHostPort(r.RemoteAddr)
	if err != nil {
		host = r.RemoteAddr
	}
	ip := net.ParseIP(host)
	return ip != nil && ip.IsLoopback()
}

func (s *Server) localPlayers(r *http.Request) []string {
	if !localRequest(r) {
		return nil
	}
	out, _, err := s.reg.CallOne(contracts.CapPlaybackLocal, nil)
	if err != nil {
		return nil
	}
	players, _ := out.(pluginlocalplay.PlayersOutput)
	return players.Players
}

func (s *Server) handleLocalPlay(w http.ResponseWriter, r *http.Request, _ auth.Verified) {
	writeJSON(w, 200, struct {
		Players []string `json:"players"`
	}{Players: s.localPlayers(r)})
}

func (s *Server) handlePlayLocal(w http.ResponseWriter, r *http.Request, v auth.Verified) {
	if !localRequest(r) {
		writeErr(w, 403, "local playback requires the browser and server on the same machine")
		return
	}
	var in struct {
		Player string `json:"player"`
	}
	if r.ContentLength > 0 && !s.decode(w, r, &in) {
		return
	}
	it, ok := s.catGet(r.PathValue("id"))
	if !ok {
		writeErr(w, 404, "unknown item")
		return
	}
	language := ""
	if u, ok := s.auth.Get(v.UserID); ok {
		language = u.PreferredLanguage
	}
	out, _, err := s.reg.CallOne(contracts.CapPlaybackLocal, pluginlocalplay.PlayInput{
		Player:   in.Player,
		UserID:   v.UserID,
		Language: language,
		Item:     it,
	})
	if err != nil {
		code := 500
		if ce, ok := err.(*core.Error); ok {
			switch ce.Code {
			case "invalid-message":
				code = 400
			case "busy", "not-found":
				code = 409
			case "dependency-unavailable":
				code = 503
			}
		}
		writeErr(w, code, err.Error())
		return
	}
	writeJSON(w, 200, out)
}
