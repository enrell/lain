package comic

import (
	"os"
	"sync"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/core"
)

// ID is the built-in comic archive provider id.
const ID = "lain-comic-archive"

// maxCached bounds the in-memory page index cache.
const maxCached = 64

type cacheKey struct {
	path  string
	mtime int64
	size  int64
}

// Provider serves lain.comic.pages@1. Indexes are cached by file
// identity (path, mtime, size), so a replaced archive re-indexes itself.
type Provider struct {
	mu    sync.Mutex
	cache map[cacheKey]contracts.ComicPages
}

func (*Provider) ID() string             { return ID }
func (*Provider) Capabilities() []string { return []string{contracts.CapComicPages} }
func (*Provider) Health() error          { return nil }

func (p *Provider) Invoke(cap string, input any) (any, error) {
	if cap != contracts.CapComicPages {
		return nil, &core.Error{Code: "invalid-message", Msg: "unsupported cap " + cap}
	}
	in, ok := input.(contracts.ComicPagesInput)
	if !ok {
		return nil, &core.Error{Code: "invalid-message", Msg: "ComicPagesInput required"}
	}
	if in.Path == "" {
		return nil, &core.Error{Code: "invalid-message", Msg: "path required"}
	}
	fi, err := os.Stat(in.Path)
	if err != nil {
		return nil, &core.Error{Code: "dependency-unavailable", Msg: "archive unavailable"}
	}
	key := cacheKey{in.Path, fi.ModTime().UnixNano(), fi.Size()}
	p.mu.Lock()
	if v, ok := p.cache[key]; ok {
		p.mu.Unlock()
		return v, nil
	}
	p.mu.Unlock()

	es, err := list(in.Path)
	if err != nil {
		return nil, err
	}
	out := contracts.ComicPages{Format: Format(in.Path), Pages: make([]contracts.ComicPage, len(es))}
	for i, e := range es {
		out.Pages[i] = contracts.ComicPage{Index: i, Name: e.name, Mime: MimeOf(e.name), Size: e.size}
	}
	// Header probing opens every entry; only the native reader can do that
	// cheaply. External formats leave sizes to the client.
	if out.Format == "cbz" {
		probeZip(in.Path, out.Pages)
	}
	p.mu.Lock()
	if p.cache == nil || len(p.cache) >= maxCached {
		p.cache = map[cacheKey]contracts.ComicPages{}
	}
	p.cache[key] = out
	p.mu.Unlock()
	return out, nil
}
