package kv

// Acquisition buckets (docs/slices/acquisition.md, A-12). Declared here
// and created by internal/acquire, so the shared bucket list in Open is
// not edited.
var (
	BAcqIndexers = []byte("acq_indexers")
	BAcqGrabs    = []byte("acq_grabs")
	BAcqSettings = []byte("acq_settings")
	// BAcqTorrents holds engine resume state per info hash: the raw
	// metainfo and the verified-piece bitfield.
	BAcqTorrents = []byte("acq_torrents")
	// Phase 2 (automation): monitored titles, quality profiles and the
	// blocklist of failed releases.
	BAcqMonitored = []byte("acq_monitored")
	BAcqProfiles  = []byte("acq_profiles")
	BAcqBlocklist = []byte("acq_blocklist")
	// BAcqQuality is the import ledger: library path → resolution.
	BAcqQuality = []byte("acq_quality")
	// Phase 3 (subtitles): provider accounts, the ledger of sidecars
	// Lain wrote, and provider files refused by the sync check.
	BAcqSubProviders = []byte("acq_subproviders")
	BAcqSubtitles    = []byte("acq_subtitles")
	BAcqSubBlock     = []byte("acq_subblock")
)

// AcquireBuckets lists every acquisition bucket.
func AcquireBuckets() [][]byte {
	return [][]byte{BAcqIndexers, BAcqGrabs, BAcqSettings, BAcqTorrents, BAcqMonitored, BAcqProfiles, BAcqBlocklist, BAcqQuality, BAcqSubProviders, BAcqSubtitles, BAcqSubBlock}
}
