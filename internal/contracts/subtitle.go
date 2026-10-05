package contracts

// CapSubtitle searches subtitle providers (docs/slices/acquisition.md,
// A-30). Ordered-many: a provider that does not serve the configured
// provider kind declines with "unsupported-provider" — the D-114
// pattern. Providers return download links; the core fetches the bytes.
const CapSubtitle = "lain.subtitle@1"

// Subtitle provider kinds.
const SubtitleOpenSubtitles = "opensubtitles"

// SubtitleProvider is one configured provider account. APIKey and
// Password are credentials: stored server-side, carried in contract
// input, never logged, and cleared by Public().
type SubtitleProvider struct {
	ID          string `json:"id"`
	Name        string `json:"name"`
	Kind        string `json:"kind"`
	BaseURL     string `json:"base_url,omitempty"` // empty = the provider's public API
	APIKey      string `json:"api_key,omitempty"`
	Username    string `json:"username,omitempty"`
	Password    string `json:"password,omitempty"`
	Enabled     bool   `json:"enabled"`
	Priority    int    `json:"priority"`
	LastCheckAt int64  `json:"last_check_at,omitempty"`
	LastError   string `json:"last_error,omitempty"`
	CreatedAt   int64  `json:"created_at"`
}

// Public is the client-safe view.
func (p SubtitleProvider) Public() SubtitleProvider {
	p.APIKey, p.Password = "", ""
	return p
}

// SubtitleSearchInput is one search: by file hash and/or by title.
// Languages are ISO 639-2/T codes (D-071's form).
type SubtitleSearchInput struct {
	Provider  SubtitleProvider `json:"provider"`
	Hash      string           `json:"hash,omitempty"`
	Size      int64            `json:"size,omitempty"`
	Query     string           `json:"query,omitempty"`
	Movie     bool             `json:"movie,omitempty"`
	Year      int              `json:"year,omitempty"`
	Season    int              `json:"season,omitempty"`
	Episode   int              `json:"episode,omitempty"`
	Languages []string         `json:"languages"`
}

// SubtitleCandidate is one subtitle a provider offers.
type SubtitleCandidate struct {
	ProviderID string `json:"provider_id"`
	FileID     string `json:"file_id"`
	Language   string `json:"language"`         // ISO 639-2/T
	Region     string `json:"region,omitempty"` // the provider's tag when it carries a region ("pt-BR")
	Release    string `json:"release,omitempty"`
	FileName   string `json:"file_name,omitempty"`
	HashMatch  bool   `json:"hash_match,omitempty"`
	HI         bool   `json:"hi,omitempty"`
	Forced     bool   `json:"forced,omitempty"`
	Downloads  int    `json:"downloads,omitempty"`
	// Season and Episode echo the provider's own numbering when given.
	Season  int `json:"season,omitempty"`
	Episode int `json:"episode,omitempty"`
}

// SubtitleSearchOutput carries candidates.
type SubtitleSearchOutput struct {
	Candidates []SubtitleCandidate `json:"candidates"`
}

// SubtitleDownloadInput asks for a download link.
type SubtitleDownloadInput struct {
	Provider SubtitleProvider `json:"provider"`
	FileID   string           `json:"file_id"`
}

// SubtitleDownloadOutput is a short-lived link the core fetches.
type SubtitleDownloadOutput struct {
	Link      string `json:"link"`
	FileName  string `json:"file_name,omitempty"`
	Remaining int    `json:"remaining"` // downloads left today; -1 = unknown
}
