// Package localplay serves lain.playback.local@1 (D-072, D-076): the
// same-machine playback decision — which players exist, what the
// playlist is, where it resumes, when subtitles stay off — lives in a
// provider, so swapping it changes local playback. The process engine
// (mpv/VLC spawning, progress sampling) stays in internal/localplay;
// this provider owns the policy on top of it and reaches catalog,
// userstate and probe through the registry.
package localplay

import (
	"errors"
	"log/slog"
	"os"
	"slices"
	"strings"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/core"
	engine "github.com/enrell/lain/internal/localplay"
	"github.com/enrell/lain/internal/plugins/catalog"
	"github.com/enrell/lain/internal/plugins/userstate"
)

const ID = "lain-playback-local"

// Provider supervises same-machine playback sessions.
type Provider struct {
	Reg *core.Registry
	// Mgr is the process engine; the gateway owns its lifecycle
	// (Close kills running players) and the test seams on it.
	Mgr *engine.Manager

	log *slog.Logger
}

// SetLogger points the provider's diagnostics at the server logger.
func (p *Provider) SetLogger(l *slog.Logger) {
	if l != nil {
		p.log = l
	}
}

func (p *Provider) logger() *slog.Logger {
	if p.log != nil {
		return p.log
	}
	return slog.New(slog.DiscardHandler)
}

func (Provider) ID() string             { return ID }
func (Provider) Capabilities() []string { return []string{contracts.CapPlaybackLocal} }

func (p Provider) Health() error {
	if p.Mgr == nil {
		return &core.Error{Code: "dependency-unavailable", Msg: "localplay has no engine"}
	}
	return nil
}

// PlayersOutput advertises which players can launch right now.
type PlayersOutput struct {
	Players []string `json:"players"`
}

// PlayInput starts one session for an item: the provider resolves the
// playlist (a series plays from this episode onward, D-056), resume
// positions and subtitle policy itself.
type PlayInput struct {
	Player   string                `json:"player,omitempty"`
	UserID   string                `json:"user_id"`
	Language string                `json:"language,omitempty"`
	Item     contracts.CatalogItem `json:"item"`
}

// PlayOutput reports the session that started.
type PlayOutput struct {
	Status string `json:"status"`
	Player string `json:"player"`
	Count  int    `json:"count"`
}

func (p Provider) Invoke(cap string, input any) (any, error) {
	if cap != contracts.CapPlaybackLocal {
		return nil, &core.Error{Code: "invalid-message", Msg: "unsupported cap " + cap}
	}
	switch in := input.(type) {
	case nil:
		return PlayersOutput{Players: p.Mgr.Players()}, nil
	case PlayInput:
		return p.play(in)
	default:
		return nil, &core.Error{Code: "invalid-message", Msg: "localplay input required"}
	}
}

func (p Provider) play(in PlayInput) (PlayOutput, error) {
	players := p.Mgr.Players()
	if len(players) == 0 {
		return PlayOutput{}, &core.Error{Code: "dependency-unavailable", Msg: "no local player: install mpv or VLC on the server machine inside a graphical session"}
	}
	player := in.Player
	if player == "" {
		player = players[0]
	}
	if !slices.Contains(players, player) {
		return PlayOutput{}, &core.Error{Code: "invalid-message", Msg: "player must be one of: " + strings.Join(players, ", ")}
	}
	items, err := p.queue(in.Item)
	if err != nil {
		return PlayOutput{}, err
	}
	entries := make([]engine.Entry, 0, len(items))
	for _, item := range items {
		e := engine.Entry{ItemID: item.ID, Path: item.FilePath}
		if prog, ok := p.progress(in.UserID, item.ID); ok && !prog.Completed {
			e.Resume = prog.PositionSec
		}
		if player == "vlc" && in.Language != "" {
			e.SubsOff = p.subsOff(item.FilePath, in.Language)
		}
		entries = append(entries, e)
	}
	err = p.Mgr.Play(engine.PlayRequest{
		Player:   player,
		Entries:  entries,
		Language: in.Language,
		Save: func(itemID string, pos, dur float64) {
			p.saveProgress(in.UserID, itemID, pos, dur)
		},
		Done: func(err error) {
			if err != nil {
				p.logger().Warn("local playback ended abnormally", "err", err.Error())
			}
		},
	})
	if err != nil {
		if errors.Is(err, engine.ErrBusy) {
			return PlayOutput{}, &core.Error{Code: "busy", Msg: err.Error()}
		}
		if errors.Is(err, engine.ErrNoPlayer) {
			return PlayOutput{}, &core.Error{Code: "dependency-unavailable", Msg: err.Error()}
		}
		return PlayOutput{}, err
	}
	return PlayOutput{Status: "playing", Player: player, Count: len(entries)}, nil
}

// queue resolves the playlist for an item: a series becomes the
// catalog's ordered episode list from this episode onward (D-056),
// with missing entries and vanished files skipped (D-068).
func (p Provider) queue(start contracts.CatalogItem) ([]contracts.CatalogItem, error) {
	var items []contracts.CatalogItem
	if start.Episode > 0 {
		if eps := p.episodes(start.ID); len(eps) > 0 {
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
		return nil, &core.Error{Code: "not-found", Msg: "the file is no longer on disk"}
	}
	return existing, nil
}

// episodes reads the title's ordered list through the catalog
// capability; a catalog failure leaves the queue a single file.
func (p Provider) episodes(id string) []contracts.CatalogItem {
	if p.Reg == nil {
		return nil
	}
	out, _, err := p.Reg.CallOne(contracts.CapCatalogRead, catalog.EpisodesInput{ID: id})
	if err != nil {
		return nil
	}
	eps, _ := out.([]contracts.CatalogItem)
	return eps
}

// progress reads resume state through the userstate capability.
func (p Provider) progress(userID, itemID string) (contracts.Progress, bool) {
	if p.Reg == nil {
		return contracts.Progress{}, false
	}
	out, _, err := p.Reg.CallOne(contracts.CapUserProgress, userstate.GetInput{UserID: userID, ItemID: itemID})
	if err != nil {
		return contracts.Progress{}, false
	}
	prog, ok := out.(contracts.Progress)
	return prog, ok
}

// saveProgress writes measured positions through the userstate
// capability — the same document the browser's own progress writes.
func (p Provider) saveProgress(userID, itemID string, pos, dur float64) {
	if p.Reg == nil {
		return
	}
	_, _, _ = p.Reg.CallOne(contracts.CapUserProgress, userstate.PutInput{
		UserID: userID,
		Progress: contracts.Progress{
			ItemID:      itemID,
			PositionSec: pos,
			DurationSec: dur,
			Completed:   pos/dur >= .95,
		},
	})
}

// subsOff probes the file's streams once to decide VLC's subtitle
// policy: matching audio keeps subs off, a matching subtitle stays
// selectable, and untagged media keeps VLC's own defaults.
func (p Provider) subsOff(path, language string) bool {
	if p.Reg == nil {
		return false
	}
	out, _, err := p.Reg.CallOne(contracts.CapMediaProbe, contracts.MediaProbeRequest{FilePath: path})
	if err != nil {
		return false
	}
	info, ok := out.(contracts.MediaInfo)
	if !ok {
		return false
	}
	return engine.SubtitleOff(info.Streams, language)
}

