package contracts

// Capability names for the tracking-list domain (D-078..D-081).
const (
	// CapListRead / CapListWrite own the per-user media tracking list
	// (D-078): standalone entries that exist with or without library
	// files, so manga, comics and other non-played types share one
	// list with anime, movies and series.
	CapListRead  = "lain.list.read@1"
	CapListWrite = "lain.list.write@1"
	// CapListAccount owns linked external list accounts: per-user
	// platform credentials and sync bookkeeping (D-079).
	CapListAccount = "lain.list.account@1"
	// CapListLink is the platform connector surface, ordered-many.
	// A provider that does not serve the requested platform declines
	// with the typed code "unsupported-platform" and the next bound
	// provider is tried — the identify pattern applied to links.
	CapListLink = "lain.listlink@1"
	// CapIntegrationSettings stores operator-provided credentials for
	// external platforms in the meta bucket (D-080). Exactly one
	// provider owns it; secrets never leave the server.
	CapIntegrationSettings = "lain.settings.integrations@1"
)

// Normalized tracking statuses shared by every platform. Providers map
// their native vocabulary onto these values on the way in.
const (
	ListStatusCurrent   = "current"
	ListStatusPlanning  = "planning"
	ListStatusCompleted = "completed"
	ListStatusPaused    = "paused"
	ListStatusDropped   = "dropped"
	ListStatusRepeating = "repeating"
)

// Broad media kinds a list entry can carry. Platforms map onto these;
// anime stays anime even when the remote calls a title a movie.
const (
	ListMediaAnime  = "anime"
	ListMediaManga  = "manga"
	ListMediaMovie  = "movie"
	ListMediaSeries = "series"
	ListMediaComic  = "comic"
)

// ListEntry is one tracked media entry on a user's list. v1 entries are
// platform-owned imports (D-081): the remote state is authoritative and
// a sync replaces them wholesale per platform.
type ListEntry struct {
	ID            string `json:"id"`
	UserID        string `json:"user_id"`
	Platform      string `json:"platform"`
	RemoteID      string `json:"remote_id"`
	MediaType     string `json:"media_type"`
	Format        string `json:"format,omitempty"`
	Title         string `json:"title"`
	Cover         string `json:"cover,omitempty"`
	Status        string `json:"status"`
	Progress      int    `json:"progress"`
	ProgressTotal int    `json:"progress_total,omitempty"`
	// ProgressVolumes is the secondary count manga platforms report
	// alongside chapters; 0 when the platform tracks no volumes.
	ProgressVolumes int     `json:"progress_volumes,omitempty"`
	Score           float64 `json:"score,omitempty"`
	Repeat          int     `json:"repeat,omitempty"`
	Notes           string  `json:"notes,omitempty"`
	StartedAt       string  `json:"started_at,omitempty"`
	CompletedAt     string  `json:"completed_at,omitempty"`
	UpdatedAt       int64   `json:"updated_at"`
}

// LinkedAccount is a user's connection to an external list platform
// (D-079). Token is a credential: it persists under json:"token" but
// Public() clears it, the same shape auth.User uses for PassHash.
type LinkedAccount struct {
	UserID         string `json:"user_id"`
	Platform       string `json:"platform"`
	RemoteUserID   string `json:"remote_user_id"`
	RemoteUsername string `json:"remote_username"`
	Token          string `json:"token,omitempty"`
	TokenExpiresAt int64  `json:"token_expires_at,omitempty"`
	LinkedAt       int64  `json:"linked_at"`
	LastSyncAt     int64  `json:"last_sync_at,omitempty"`
	LastSyncError  string `json:"last_sync_error,omitempty"`
	EntryCount     int    `json:"entry_count,omitempty"`
}

// Public returns the account safe to serialize to a client: the token
// never crosses the wire (D-079).
func (a LinkedAccount) Public() LinkedAccount {
	a.Token = ""
	return a
}

// TokenExpired reports whether the stored credential is past its remote
// lifetime; zero means the platform reported no expiry.
func (a LinkedAccount) TokenExpired(now int64) bool {
	return a.TokenExpiresAt > 0 && now >= a.TokenExpiresAt
}

// ListSyncStats reports one platform import: entries written and
// entries removed because the remote no longer lists them.
type ListSyncStats struct {
	Upserted int `json:"upserted"`
	Removed  int `json:"removed"`
}

// LinkAuthorizeInput asks a platform provider for the URL that starts
// its OAuth flow. The gateway builds the signed state; the provider
// only knows how to place it in its own URL shape.
type LinkAuthorizeInput struct {
	Platform    string `json:"platform"`
	ClientID    string `json:"client_id"`
	RedirectURI string `json:"redirect_uri"`
	State       string `json:"state"`
}

// LinkAuthorizeOutput carries the URL the client opens.
type LinkAuthorizeOutput struct {
	URL string `json:"url"`
}

// LinkExchangeInput trades an authorization code for a credential.
// Secrets flow inside the contract input — the provider holds no
// operator state (D-006/D-080).
type LinkExchangeInput struct {
	Platform     string `json:"platform"`
	ClientID     string `json:"client_id"`
	ClientSecret string `json:"client_secret"`
	Code         string `json:"code"`
	RedirectURI  string `json:"redirect_uri"`
}

// LinkedIdentity is the remote account an exchange produced. Token is
// trusted-plane data handed straight to the account store.
type LinkedIdentity struct {
	Token          string `json:"token"`
	TokenExpiresAt int64  `json:"token_expires_at,omitempty"`
	RemoteUserID   string `json:"remote_user_id"`
	RemoteUsername string `json:"remote_username"`
}

// LinkFetchInput pulls the remote list for one linked account.
type LinkFetchInput struct {
	Platform     string `json:"platform"`
	Token        string `json:"token"`
	RemoteUserID string `json:"remote_user_id"`
}

// LinkFetchOutput is the remote list as entries; the store stamps user,
// platform and ids on its side.
type LinkFetchOutput struct {
	Entries []ListEntry `json:"entries"`
}

// IntegrationSettings is the operator's external-platform credential
// set (D-080). The secret persists but never serializes back out: the
// gateway reports only whether one is stored.
type IntegrationSettings struct {
	AniListClientID     string `json:"anilist_client_id,omitempty"`
	AniListClientSecret string `json:"anilist_client_secret,omitempty"`
}
