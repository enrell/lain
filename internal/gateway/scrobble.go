package gateway

import (
	"encoding/json"
	"net/http"
	"time"

	"github.com/enrell/lain/internal/auth"
	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/plugins/catalog"
	"github.com/enrell/lain/internal/plugins/list"
)

// Scrobble pushes finished episodes back to a linked platform. It is
// opt-in per account (LinkedAccount.Scrobble) because it writes to the
// user's remote list, and deliberately conservative: it only advances
// an entry the user already tracks, matched by exact normalized title,
// and never regresses progress or reopens a completed entry.

// scrobbleDecision is what to write for one finished episode.
type scrobbleDecision struct {
	RemoteID string
	Progress int
	Status   string
}

// planScrobble decides whether finishing `item` should update one of
// the user's platform entries. Later seasons are skipped: platforms
// list each season as its own entry, and a per-season episode number
// against the wrong entry would corrupt the list.
func planScrobble(item contracts.CatalogItem, platform string, entries []contracts.ListEntry) (scrobbleDecision, bool) {
	if item.Episode <= 0 || item.Season > 1 || item.Kind == "movie" {
		return scrobbleDecision{}, false
	}
	key := catalog.TitleKey(item.Title)
	var match *contracts.ListEntry
	for i := range entries {
		e := &entries[i]
		if e.Platform != platform || e.MediaType != contracts.ListMediaAnime || catalog.TitleKey(e.Title) != key {
			continue
		}
		if match != nil {
			return scrobbleDecision{}, false // ambiguous: never guess
		}
		match = e
	}
	if match == nil {
		return scrobbleDecision{}, false
	}
	switch match.Status {
	case contracts.ListStatusCompleted, contracts.ListStatusRepeating:
		return scrobbleDecision{}, false
	}
	if item.Episode <= match.Progress || (match.ProgressTotal > 0 && item.Episode > match.ProgressTotal) {
		return scrobbleDecision{}, false
	}
	status := contracts.ListStatusCurrent
	if match.ProgressTotal > 0 && item.Episode == match.ProgressTotal {
		status = contracts.ListStatusCompleted
	}
	return scrobbleDecision{RemoteID: match.RemoteID, Progress: item.Episode, Status: status}, true
}

// scrobble runs synchronously; the progress handler calls it on its own
// goroutine so a slow platform never delays playback reporting.
func (s *Server) scrobble(userID, itemID string) {
	item, ok := s.catGet(itemID)
	if !ok {
		return
	}
	out, _, err := s.reg.CallOne(contracts.CapListAccount, list.ListAccountsInput{UserID: userID})
	if err != nil {
		s.logger().Warn("scrobble accounts failed", "user", userID, "err", err.Error())
		return
	}
	accounts, _ := out.([]contracts.LinkedAccount)
	for _, a := range accounts {
		if !a.Scrobble || a.Token == "" {
			continue
		}
		s.scrobbleTo(a, item)
	}
}

func (s *Server) scrobbleTo(a contracts.LinkedAccount, item contracts.CatalogItem) {
	out, _, err := s.reg.CallOne(contracts.CapListRead, list.ListInput{UserID: a.UserID, Type: contracts.ListMediaAnime})
	if err != nil {
		s.logger().Warn("scrobble list read failed", "platform", a.Platform, "err", err.Error())
		return
	}
	entries, _ := out.([]contracts.ListEntry)
	d, ok := planScrobble(item, a.Platform, entries)
	if !ok {
		return
	}
	if a.TokenExpired(time.Now().Unix()) {
		s.logger().Warn("scrobble skipped: token expired", "platform", a.Platform, "user", a.UserID)
		return
	}
	if _, err := s.listlinkInvoke(contracts.LinkPushInput{
		Platform: a.Platform, Token: a.Token, RemoteID: d.RemoteID, Progress: d.Progress, Status: d.Status,
	}); err != nil {
		s.logger().Warn("scrobble failed", "platform", a.Platform, "user", a.UserID, "err", err.Error())
		return
	}
	s.logger().Info("scrobbled", "platform", a.Platform, "user", a.UserID, "progress", d.Progress, "status", d.Status)
	s.updateLocalEntry(a, d)
}

// updateLocalEntry keeps the stored copy in step with what the platform
// now holds, so the next finished episode compares against it without
// waiting for a sync. PutPlatform replaces a platform's set wholesale,
// so the read-modify-write runs under the sync lock.
func (s *Server) updateLocalEntry(a contracts.LinkedAccount, d scrobbleDecision) {
	linkSyncMu.Lock()
	defer linkSyncMu.Unlock()
	out, _, err := s.reg.CallOne(contracts.CapListRead, list.ListInput{UserID: a.UserID})
	if err != nil {
		s.logger().Warn("scrobble local update failed", "err", err.Error())
		return
	}
	all, _ := out.([]contracts.ListEntry)
	var mine []contracts.ListEntry
	for _, e := range all {
		if e.Platform != a.Platform {
			continue
		}
		if e.RemoteID == d.RemoteID {
			e.Progress, e.Status = d.Progress, d.Status
		}
		mine = append(mine, e)
	}
	if _, _, err := s.reg.CallOne(contracts.CapListWrite, list.PutPlatformInput{UserID: a.UserID, Platform: a.Platform, Entries: mine}); err != nil {
		s.logger().Warn("scrobble local update failed", "err", err.Error())
	}
}

// handleLinkPatch changes per-account preferences; today only scrobble.
func (s *Server) handleLinkPatch(w http.ResponseWriter, r *http.Request, v auth.Verified) {
	var in struct {
		Scrobble *bool `json:"scrobble"`
	}
	defer r.Body.Close()
	if err := json.NewDecoder(r.Body).Decode(&in); err != nil || in.Scrobble == nil {
		writeErr(w, http.StatusBadRequest, "scrobble (boolean) required")
		return
	}
	account, ok := s.getAccount(v.UserID, r.PathValue("platform"))
	if !ok {
		writeErr(w, http.StatusNotFound, "no "+r.PathValue("platform")+" account linked")
		return
	}
	account.Scrobble = *in.Scrobble
	s.putAccount(account)
	writeJSON(w, http.StatusOK, publicLink(account, time.Now().Unix()))
}
