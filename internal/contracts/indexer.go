package contracts

// CapIndexer searches release indexers (docs/slices/acquisition.md,
// A-7). Ordered-many: a provider that does not speak the indexer's
// protocol declines with the typed code "unsupported-protocol" and the
// next bound provider is tried — the lain.listlink@1 pattern.
const CapIndexer = "lain.indexer@1"

// Indexer protocols.
const (
	IndexerTorznab = "torznab"
	IndexerNewznab = "newznab"
)

// Indexer is one configured source. APIKey is a credential: it persists
// server-side and travels inside contract input, and Public() clears it
// so it never reaches a client or a log line (D-026).
type Indexer struct {
	ID         string `json:"id"`
	Name       string `json:"name"`
	Protocol   string `json:"protocol"`
	URL        string `json:"url"`
	APIKey     string `json:"api_key,omitempty"`
	Categories []int  `json:"categories"`
	Enabled    bool   `json:"enabled"`
	Priority   int    `json:"priority"`
	// MinIntervalMs is the minimum time between two requests (A-8).
	MinIntervalMs int          `json:"min_interval_ms"`
	Caps          *IndexerCaps `json:"caps,omitempty"`
	LastCheckAt   int64        `json:"last_check_at,omitempty"`
	LastError     string       `json:"last_error,omitempty"`
	CreatedAt     int64        `json:"created_at"`
}

// Public is the client-safe view.
func (i Indexer) Public() Indexer {
	i.APIKey = ""
	return i
}

// IndexerCaps is what a t=caps probe reported.
type IndexerCaps struct {
	Search      bool              `json:"search"`
	TVSearch    bool              `json:"tv_search"`
	MovieSearch bool              `json:"movie_search"`
	BookSearch  bool              `json:"book_search"`
	Categories  []IndexerCategory `json:"categories"`
	Limit       int               `json:"limit,omitempty"`
}

// IndexerCategory is a Newznab category id and name.
type IndexerCategory struct {
	ID   int    `json:"id"`
	Name string `json:"name"`
}

// IndexerCapsInput probes an indexer (the health check).
type IndexerCapsInput struct {
	Indexer Indexer `json:"indexer"`
}

// Search kinds map onto Newznab functions: tv → t=tvsearch,
// movie → t=movie, book → t=book, anything else → t=search.
const (
	SearchGeneric = ""
	SearchTV      = "tv"
	SearchMovie   = "movie"
	SearchBook    = "book"
)

// IndexerSearchInput is one query against one indexer.
type IndexerSearchInput struct {
	Indexer Indexer `json:"indexer"`
	Query   string  `json:"query"`
	Kind    string  `json:"kind,omitempty"`
	Season  int     `json:"season,omitempty"`
	Episode int     `json:"episode,omitempty"`
	Year    int     `json:"year,omitempty"`
	// Categories overrides the indexer's own list when non-empty.
	Categories []int `json:"categories,omitempty"`
	Limit      int   `json:"limit,omitempty"`
}

// Result protocols.
const (
	ProtocolTorrent = "torrent"
	ProtocolUsenet  = "usenet"
)

// SearchResult is one release an indexer offered.
type SearchResult struct {
	IndexerID   string `json:"indexer_id"`
	Protocol    string `json:"protocol"`
	Title       string `json:"title"`
	GUID        string `json:"guid,omitempty"`
	Link        string `json:"link,omitempty"`   // .torrent or .nzb download URL
	Magnet      string `json:"magnet,omitempty"` // torrent only
	InfoHash    string `json:"info_hash,omitempty"`
	Size        int64  `json:"size"`
	Seeders     int    `json:"seeders"`
	Peers       int    `json:"peers"`
	PublishedAt int64  `json:"published_at,omitempty"`
	Categories  []int  `json:"categories,omitempty"`
	CommentsURL string `json:"comments_url,omitempty"`
	// Freeleech marks a download volume factor of 0.
	Freeleech bool `json:"freeleech,omitempty"`
}

// IndexerSearchOutput carries results.
type IndexerSearchOutput struct {
	Results []SearchResult `json:"results"`
}
