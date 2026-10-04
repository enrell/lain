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
)

// AcquireBuckets lists every acquisition bucket.
func AcquireBuckets() [][]byte {
	return [][]byte{BAcqIndexers, BAcqGrabs, BAcqSettings, BAcqTorrents}
}
