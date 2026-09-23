package gateway

import (
	"errors"
	"net"
	"net/http"
	"os"
	"slices"
	"strings"

	"github.com/enrell/lain/internal/auth"
	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/localplay"
	"github.com/enrell/lain/internal/plugins/userstate"
)

// Local playback (D-072): when the browser sits on the same machine as a
// natively running server, the server can spawn mpv/VLC itself — the
// files are already local, so no token or stream URL ever crosses a
// link. Remote browsers get an empty capability and a 403 on play.

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
	if !localRequest(r) || s.local == nil {
		return nil
	}
	return s.local.Players()
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
	it, ok := s.cat.Get(r.PathValue("id"))
	if !ok {
		writeErr(w, 404, "unknown item")
		return
	}
	items, err := s.localQueue(it)
	if err != nil {
		writeErr(w, 409, err.Error())
		return
	}
	players := s.local.Players()
	if len(players) == 0 {
		writeErr(w, 503, "no local player: install mpv or VLC on the server machine inside a graphical session")
		return
	}
	player := in.Player
	if player == "" {
		player = players[0]
	}
	if !slices.Contains(players, player) {
		writeErr(w, 400, "player must be one of: "+strings.Join(players, ", "))
		return
	}
	language := ""
	if u, ok := s.auth.Get(v.UserID); ok {
		language = u.PreferredLanguage
	}
	entries := make([]localplay.Entry, 0, len(items))
	for _, item := range items {
		e := localplay.Entry{ItemID: item.ID, Path: item.FilePath}
		if p, ok := s.ustate.Get(v.UserID, item.ID); ok && !p.Completed {
			e.Resume = p.PositionSec
		}
		if player == "vlc" && language != "" {
			e.SubsOff = s.vlcSubsOff(item.FilePath, language)
		}
		entries = append(entries, e)
	}
	err = s.local.Play(localplay.PlayRequest{
		Player:   player,
		Entries:  entries,
		Language: language,
		Save: func(itemID string, pos, dur float64) {
			_, _ = s.ustate.Put(userstate.PutInput{UserID: v.UserID, Progress: contracts.Progress{
				ItemID:      itemID,
				PositionSec: pos,
				DurationSec: dur,
				Completed:   pos/dur >= .95,
			}})
		},
		Done: func(err error) {
			if err != nil {
				s.logger().Warn("local playback ended abnormally", "err", err.Error())
			}
		},
	})
	if err != nil {
		code := 500
		if errors.Is(err, localplay.ErrBusy) {
			code = 409
		} else if errors.Is(err, localplay.ErrNoPlayer) {
			code = 503
		}
		writeErr(w, code, err.Error())
		return
	}
	writeJSON(w, 200, map[string]any{"status": "playing", "player": player, "count": len(entries)})
}

// localQueue resolves the playback list for an item: a series becomes the
// catalog's ordered episode list from this episode onward (D-056), with
// missing entries and vanished files skipped (D-068). Anything else is a
// single-file playlist.
func (s *Server) localQueue(start contracts.CatalogItem) ([]contracts.CatalogItem, error) {
	var items []contracts.CatalogItem
	if start.Episode > 0 {
		if eps, ok := s.cat.Episodes(start.ID); ok {
			found := false
			for _, e := range eps {
				if e.ID == start.ID {
					found = true
				}
				if found && !e.Missing {
					items = append(items, e)
				}
			}
		}
	}
	if len(items) == 0 {
		items = []contracts.CatalogItem{start}
	}
	existing := items[:0]
	for _, item := range items {
		if _, err := os.Stat(item.FilePath); err == nil {
			existing = append(existing, item)
		}
	}
	if len(existing) == 0 {
		return nil, errors.New("the file is no longer on disk")
	}
	return existing, nil
}

// vlcSubsOff probes the file's streams once to decide VLC's subtitle
// policy: matching audio keeps subs off, a matching subtitle stays
// selectable, and untagged media keeps VLC's own defaults.
func (s *Server) vlcSubsOff(path, language string) bool {
	out, _, err := s.reg.CallOne(contracts.CapMediaProbe, contracts.MediaProbeRequest{FilePath: path})
	if err != nil {
		return false
	}
	info, ok := out.(contracts.MediaInfo)
	if !ok {
		return false
	}
	return localplay.SubtitleOff(info.Streams, language)
}
