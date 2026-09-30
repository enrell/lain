package gateway

import (
	"fmt"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/core"
	"github.com/enrell/lain/internal/plugins/catalog"
	"github.com/enrell/lain/internal/plugins/identify"
	"github.com/enrell/lain/internal/plugins/search"
	"github.com/enrell/lain/internal/plugins/userstate"
)

// The gateway composes built-in providers without importing their
// internals beyond the public Invoke surface. These aliases keep
// server.go readable while the concrete policy lives in each plugin
// package (the unit that a community plugin would replace).
type (
	identifyAnimeShim   = identify.Anime
	identifyComicShim   = identify.Comic
	identifyGenericShim = identify.Generic
)

type searchProvider struct {
	reg *core.Registry
}

func (p searchProvider) ID() string             { return search.ID }
func (p searchProvider) Capabilities() []string { return []string{contracts.CapSearchQuery} }
func (p searchProvider) Health() error          { return nil }

func (p searchProvider) Invoke(cap string, input any) (any, error) {
	out, _, err := p.reg.CallOne(contracts.CapCatalogRead, nil)
	if err != nil {
		return nil, err
	}
	items, _ := out.([]contracts.CatalogItem)
	return search.Provider{Items: func() []contracts.CatalogItem { return items }}.Invoke(cap, input)
}

func userStatePut(userID string, p contracts.Progress) any {
	return userstate.PutInput{UserID: userID, Progress: p}
}

func userStateGet(userID, itemID string) any {
	return userstate.GetInput{UserID: userID, ItemID: itemID}
}

// Catalog and userstate go through the registry like every other
// capability (D-075): the exactly-one bindings are the authority, so a
// swapped provider actually serves these reads and writes. Callers keep
// the old (value, ok) shapes; not-found maps to ok=false and any other
// failure is logged, never silent (D-011).

func notFound(err error) bool {
	e, ok := err.(*core.Error)
	return ok && e.Code == "not-found"
}

func (s *Server) catGet(id string) (contracts.CatalogItem, bool) {
	out, _, err := s.reg.CallOne(contracts.CapCatalogRead, catalog.GetInput{ID: id})
	if err != nil {
		if !notFound(err) {
			s.logger().Warn("catalog get failed", "item", id, "err", err.Error())
		}
		return contracts.CatalogItem{}, false
	}
	it, ok := out.(contracts.CatalogItem)
	return it, ok
}

func (s *Server) catList() []contracts.CatalogItem {
	out, _, err := s.reg.CallOne(contracts.CapCatalogRead, nil)
	if err != nil {
		s.logger().Warn("catalog list failed", "err", err.Error())
		return nil
	}
	items, _ := out.([]contracts.CatalogItem)
	return items
}

func (s *Server) catPage(p contracts.PageParams) (contracts.CatalogPage, error) {
	out, _, err := s.reg.CallOne(contracts.CapCatalogRead, catalog.PageInput{Page: p})
	if err != nil {
		return contracts.CatalogPage{}, err
	}
	page, ok := out.(contracts.CatalogPage)
	if !ok {
		return contracts.CatalogPage{}, fmt.Errorf("bad catalog page")
	}
	return page, nil
}

func (s *Server) catEpisodes(id string) ([]contracts.CatalogItem, bool) {
	out, _, err := s.reg.CallOne(contracts.CapCatalogRead, catalog.EpisodesInput{ID: id})
	if err != nil {
		if !notFound(err) {
			s.logger().Warn("catalog episodes failed", "item", id, "err", err.Error())
		}
		return nil, false
	}
	items, ok := out.([]contracts.CatalogItem)
	return items, ok
}

func (s *Server) catDeleteLibrary(libraryID string) (int, error) {
	out, _, err := s.reg.CallOne(contracts.CapCatalogWrite, catalog.DeleteLibraryInput{LibraryID: libraryID})
	if err != nil {
		return 0, err
	}
	removed, _ := out.(int)
	return removed, nil
}

func (s *Server) catSetMissing(id string, missing bool) (bool, error) {
	out, _, err := s.reg.CallOne(contracts.CapCatalogWrite, catalog.SetMissingInput{ID: id, Missing: missing})
	if err != nil {
		return false, err
	}
	found, _ := out.(bool)
	return found, nil
}

func (s *Server) ustateGet(userID, itemID string) (contracts.Progress, bool) {
	out, _, err := s.reg.CallOne(contracts.CapUserProgress, userstate.GetInput{UserID: userID, ItemID: itemID})
	if err != nil {
		return contracts.Progress{}, false
	}
	p, ok := out.(contracts.Progress)
	return p, ok && p.ItemID != ""
}

func (s *Server) ustatePut(userID string, p contracts.Progress) {
	if _, _, err := s.reg.CallOne(contracts.CapUserProgress, userstate.PutInput{UserID: userID, Progress: p}); err != nil {
		s.logger().Warn("progress write failed", "user", userID, "item", p.ItemID, "err", err.Error())
	}
}

func (s *Server) ustateList(userID string) ([]contracts.Progress, error) {
	out, _, err := s.reg.CallOne(contracts.CapUserProgress, userstate.ListInput{UserID: userID})
	if err != nil {
		return nil, err
	}
	list, _ := out.([]contracts.Progress)
	return list, nil
}
