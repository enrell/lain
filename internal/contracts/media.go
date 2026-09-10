// Package contracts defines the Lain media capability names and the
// JSON shapes that cross a plugin boundary.
//
// A capability name is not a credential: it names a replaceable
// behavior. Authorization to serve a capability comes from the core
// composition (see internal/core), never from knowing the string.
package contracts

// Capability names served by v0.1 plugins.
const (
	CapSourceEnumerate = "lain.source.enumerate@1"
	CapMediaIdentify   = "lain.media.identify@1"
	CapCatalogRead     = "lain.catalog.read@1"
	CapCatalogWrite    = "lain.catalog.write@1"
	CapUserProgress    = "lain.userstate.progress@1"
	CapPlaybackPlan    = "lain.playback.plan@1"
	CapSearchQuery     = "lain.search.query@1"
	CapIngestScan      = "lain.ingest.scan@1"
	CapMetadataSearch  = "lain.metadata.search@1"
	CapMetadataResolve = "lain.metadata.resolve@1"
)

// Candidate is a raw discovered file handed to identifiers.
// It carries no interpretation: parsing is the identifier's job.
// Paths stay inside the server; clients receive opaque asset refs.
type Candidate struct {
	Path      string `json:"path"`
	Size      int64  `json:"size"`
	ModTime   int64  `json:"mod_time"`
	LibraryID string `json:"library_id"`
}

// Proposal is an identifier's hypothesis about a candidate.
// Confidence is advisory in [0,1]; the ingest pipeline picks the
// first accepted proposal from the ordered binding.
type Proposal struct {
	Kind       string   `json:"kind"`
	Title      string   `json:"title"`
	Season     int      `json:"season"`
	Episode    int      `json:"episode"`
	Year       int      `json:"year"`
	Confidence float64  `json:"confidence"`
	Evidence   []string `json:"evidence"`
	PluginID   string   `json:"plugin_id"`
}

// Accepted reports whether the pipeline should take this proposal.
func (p Proposal) Accepted() bool { return p.Title != "" && p.Confidence > 0 }

// Library is a configured media root bound to a source plugin.
type Library struct {
	ID        string `json:"id"`
	Name      string `json:"name"`
	Type      string `json:"type"`
	Path      string `json:"path"`
	Source    string `json:"source"`
	CreatedAt int64  `json:"created_at"`
}

// CatalogItem is the canonical identity of one media object plus its
// provenance. Manual user edits must survive provider swaps, so every
// consolidated field records its origin.
type CatalogItem struct {
	ID         string  `json:"id"`
	LibraryID  string  `json:"library_id"`
	Kind       string  `json:"kind"`
	Title      string  `json:"title"`
	Season     int     `json:"season"`
	Episode    int     `json:"episode"`
	Year       int     `json:"year"`
	FilePath   string  `json:"file_path"`
	Size       int64   `json:"size"`
	Confidence float64 `json:"confidence"`
	Origin     string  `json:"origin"`
	Provenance string  `json:"provenance"`
	UpdatedAt  int64   `json:"updated_at"`
}

// CatalogPage is the paged envelope for catalog reads. Clients need
// the total to paginate; default limit 50, hard cap 500.
type CatalogPage struct {
	Items  []CatalogItem `json:"items"`
	Total  int           `json:"total"`
	Limit  int           `json:"limit"`
	Offset int           `json:"offset"`
}

// PageParams carries validated paging input.
type PageParams struct {
	Limit  int
	Offset int
	Sort   string // "title" (default) or "recent"
}

// NormalizePage clamps raw query input to safe bounds.
func NormalizePage(limit, offset int, sort string) PageParams {
	if limit <= 0 {
		limit = 50
	}
	if limit > 500 {
		limit = 500
	}
	if offset < 0 {
		offset = 0
	}
	if sort != "recent" {
		sort = "title"
	}
	return PageParams{Limit: limit, Offset: offset, Sort: sort}
}

// MetadataCandidate is one provider's search hit. Score is populated
// by the merger (precedence + title match), never by the provider.
type MetadataCandidate struct {
	Provider string   `json:"provider"`
	RemoteID string   `json:"remote_id"`
	Title    string   `json:"title"`
	Synonyms []string `json:"synonyms,omitempty"`
	Year     int      `json:"year,omitempty"`
	Kind     string   `json:"kind,omitempty"`
	Poster   string   `json:"poster,omitempty"`
	Score    float64  `json:"score,omitempty"`
}

// MetadataSearchInput queries providers. Dir hints local sources
// (NFO sidecars); remote providers ignore it.
type MetadataSearchInput struct {
	Query string `json:"query"`
	Kind  string `json:"kind"`
	Limit int    `json:"limit"`
	Dir   string `json:"dir,omitempty"`
}

// MetadataRecord is a resolved full entry. Artwork rides as URLs;
// bytes are the client's business (or a future artwork-cache provider).
type MetadataRecord struct {
	Provider string   `json:"provider"`
	RemoteID string   `json:"remote_id"`
	Title    string   `json:"title"`
	Synonyms []string `json:"synonyms,omitempty"`
	Year     int      `json:"year,omitempty"`
	Genres   []string `json:"genres,omitempty"`
	Synopsis string   `json:"synopsis,omitempty"`
	Poster   string   `json:"poster,omitempty"`
	Cover    string   `json:"cover,omitempty"`
	Episodes int      `json:"episodes,omitempty"`
}

// MetadataResolveInput fetches one record by provider identity.
type MetadataResolveInput struct {
	Provider string `json:"provider"`
	RemoteID string `json:"remote_id"`
}

// Enrichment is the stored overlay on a catalog item. Identity stays
// in the catalog; this document only decorates. Removing a provider
// deletes its overlays without touching identity, progress or files.
type Enrichment struct {
	ItemID    string   `json:"item_id"`
	Provider  string   `json:"provider"`
	RemoteID  string   `json:"remote_id"`
	Title     string   `json:"title"`
	Year      int      `json:"year,omitempty"`
	Genres    []string `json:"genres,omitempty"`
	Synopsis  string   `json:"synopsis,omitempty"`
	Poster    string   `json:"poster,omitempty"`
	Cover     string   `json:"cover,omitempty"`
	FetchedAt int64    `json:"fetched_at"`
}

// Progress is per-user playback state. It lives in userstate, never in
// the catalog, so reindexing media cannot destroy resume positions.
type Progress struct {
	ItemID      string  `json:"item_id"`
	UserID      string  `json:"user_id"`
	PositionSec float64 `json:"position_sec"`
	DurationSec float64 `json:"duration_sec"`
	Completed   bool    `json:"completed"`
	UpdatedAt   int64   `json:"updated_at"`
}

// PlanRequest asks the playback planner how a client should play an item.
type PlanRequest struct {
	ItemID  string `json:"item_id"`
	Client  string `json:"client"`
	Network string `json:"network"`
}

// Plan is the planner's answer. Mode is "direct" or
// "transcode-required". Asset is an opaque reference resolved by the
// data gateway, never a raw filesystem path.
type Plan struct {
	Mode      string `json:"mode"`
	Asset     string `json:"asset"`
	Profile   string `json:"profile,omitempty"`
	Session   string `json:"session,omitempty"`
	Available bool   `json:"available"`
	Reason    string `json:"reason,omitempty"`
}

// ScanStats summarizes one ingest run.
type ScanStats struct {
	Libraries    int   `json:"libraries"`
	Candidates   int   `json:"candidates"`
	Identified   int   `json:"identified"`
	Unidentified int   `json:"unidentified"`
	Errors       int   `json:"errors"`
	Pruned       int   `json:"pruned"`
	WalkErrors   int   `json:"walk_errors"`
	Dirs         int   `json:"dirs"`
	StartedAt    int64 `json:"started_at"`
	FinishedAt   int64 `json:"finished_at"`
}
