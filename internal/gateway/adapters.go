package gateway

import (
	"github.com/enrell/lain/internal/contracts"
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
	identifyGenericShim = identify.Generic
)

type searchProvider struct {
	cat interface {
		List() []contracts.CatalogItem
	}
}

func (p searchProvider) ID() string             { return search.ID }
func (p searchProvider) Capabilities() []string { return []string{contracts.CapSearchQuery} }
func (p searchProvider) Health() error          { return nil }

func (p searchProvider) Invoke(cap string, input any) (any, error) {
	return search.Provider{Items: p.cat.List}.Invoke(cap, input)
}

func userStatePut(userID string, p contracts.Progress) any {
	return userstate.PutInput{UserID: userID, Progress: p}
}

func userStateGet(userID, itemID string) any {
	return userstate.GetInput{UserID: userID, ItemID: itemID}
}
