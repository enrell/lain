package component

import (
	"fmt"
	"reflect"

	"github.com/enrell/lain/internal/contracts"
	"github.com/enrell/lain/internal/plugins/backup"
	"github.com/enrell/lain/internal/plugins/catalog"
	"github.com/enrell/lain/internal/plugins/ingest"
	"github.com/enrell/lain/internal/plugins/localplay"
	"github.com/enrell/lain/internal/plugins/metadata"
	"github.com/enrell/lain/internal/plugins/playback"
	"github.com/enrell/lain/internal/plugins/search"
	"github.com/enrell/lain/internal/plugins/settings"
	"github.com/enrell/lain/internal/plugins/source"
	"github.com/enrell/lain/internal/plugins/sourcewatch"
	"github.com/enrell/lain/internal/plugins/theme"
	"github.com/enrell/lain/internal/plugins/userstate"
)

// opSpec describes one operation inside a capability: In returns a
// pointer to the input shape for decoding (nil when the op takes no
// body), Out a pointer to the output shape (nil for no result body).
// Op names are part of the capability contract — documented in
// docs/CONTRACTS.md — so an external component in any language reads
// the same table.
type opSpec struct {
	In  func() any
	Out func() any
}

// ops is the single dispatch table for every built-in capability. It
// mirrors each provider's Invoke type-switch; a capability absent here
// cannot cross the component boundary.
var ops = map[string]map[string]opSpec{
	contracts.CapSourceEnumerate: {
		"enumerate": {In: func() any { return &source.EnumerateInput{} }, Out: func() any { return &source.EnumerateOutput{} }},
	},
	contracts.CapMediaIdentify: {
		"identify": {In: func() any { return &contracts.Candidate{} }, Out: func() any { return &contracts.Proposal{} }},
	},
	contracts.CapMediaProbe: {
		"probe": {In: func() any { return &contracts.MediaProbeRequest{} }, Out: func() any { return &contracts.MediaInfo{} }},
	},
	contracts.CapCatalogRead: {
		"list":     {In: nil, Out: func() any { return &[]contracts.CatalogItem{} }},
		"get":      {In: func() any { return &catalog.GetInput{} }, Out: func() any { return &contracts.CatalogItem{} }},
		"page":     {In: func() any { return &catalog.PageInput{} }, Out: func() any { return &contracts.CatalogPage{} }},
		"episodes": {In: func() any { return &catalog.EpisodesInput{} }, Out: func() any { return &[]contracts.CatalogItem{} }},
	},
	contracts.CapCatalogWrite: {
		"upsert":         {In: func() any { return &catalog.UpsertInput{} }, Out: func() any { return &contracts.CatalogItem{} }},
		"commit_scan":    {In: func() any { return &catalog.CommitScanInput{} }, Out: func() any { return &catalog.CommitScanOutput{} }},
		"set_missing":    {In: func() any { return &catalog.SetMissingInput{} }, Out: func() any { return new(bool) }},
		"delete_library": {In: func() any { return &catalog.DeleteLibraryInput{} }, Out: func() any { return new(int) }},
	},
	contracts.CapUserProgress: {
		"put":  {In: func() any { return &userstate.PutInput{} }, Out: func() any { return &contracts.Progress{} }},
		"get":  {In: func() any { return &userstate.GetInput{} }, Out: func() any { return &contracts.Progress{} }},
		"list": {In: func() any { return &userstate.ListInput{} }, Out: func() any { return &[]contracts.Progress{} }},
	},
	contracts.CapPlaybackPlan: {
		"plan": {In: func() any { return &playback.PlanInput{} }, Out: func() any { return &contracts.Plan{} }},
	},
	contracts.CapSearchQuery: {
		"query": {In: func() any { return &search.QueryInput{} }, Out: func() any { return &contracts.CatalogPage{} }},
	},
	contracts.CapIngestScan: {
		"scan": {In: func() any { return &ingest.ScanInput{} }, Out: func() any { return &contracts.ScanStats{} }},
	},
	contracts.CapMetadataSearch: {
		"search": {In: func() any { return &contracts.MetadataSearchInput{} }, Out: func() any { return &[]contracts.MetadataCandidate{} }},
	},
	contracts.CapMetadataResolve: {
		"resolve": {In: func() any { return &contracts.MetadataResolveInput{} }, Out: func() any { return &contracts.MetadataRecord{} }},
	},
	contracts.CapTransformThumb: {
		"generate": {In: func() any { return &contracts.ThumbnailRequest{} }, Out: func() any { return &contracts.Thumbnail{} }},
	},
	contracts.CapPlaybackTranscode: {
		"run": {In: func() any { return &contracts.TranscodeRequest{} }, Out: func() any { return &contracts.Transcode{} }},
	},
	contracts.CapPlaybackTranscodeV2: {
		"session": {In: func() any { return &contracts.TranscodeV2Request{} }, Out: func() any { return &contracts.TranscodeStatus{} }},
	},
	contracts.CapPlaybackTranscodeV3: {
		"session": {In: func() any { return &contracts.TranscodeV3Request{} }, Out: func() any { return &contracts.TranscodeV3Status{} }},
	},
	contracts.CapTranscodeSettings: {
		"get":    {In: nil, Out: func() any { return &contracts.TranscodeSettings{} }},
		"put":    {In: func() any { return &settings.PutInput{} }, Out: func() any { return &contracts.TranscodeSettings{} }},
		"ensure": {In: func() any { return &settings.EnsureInput{} }, Out: func() any { return &settings.HasOutput{} }},
		"has":    {In: func() any { return &settings.HasInput{} }, Out: func() any { return &settings.HasOutput{} }},
	},
	contracts.CapUITheme: {
		"derive": {In: func() any { return &theme.Input{} }, Out: func() any { return &theme.Palette{} }},
	},
	contracts.CapMetadataEnrich: {
		"enrich":   {In: func() any { return &metadata.EnrichInput{} }, Out: func() any { return &contracts.Enrichment{} }},
		"get":      {In: func() any { return &metadata.GetInput{} }, Out: func() any { return &contracts.Enrichment{} }},
		"batch":    {In: func() any { return &metadata.BatchInput{} }, Out: func() any { return &metadata.BatchOutput{} }},
		"delete":   {In: func() any { return &metadata.DeleteInput{} }, Out: func() any { return new(bool) }},
		"backfill": {In: func() any { return &metadata.BackfillInput{} }, Out: func() any { return &metadata.BackfillOutput{} }},
	},
	contracts.CapPlaybackLocal: {
		"players": {In: nil, Out: func() any { return &localplay.PlayersOutput{} }},
		"play":    {In: func() any { return &localplay.PlayInput{} }, Out: func() any { return &localplay.PlayOutput{} }},
	},
	contracts.CapBackupCreate: {
		"create": {In: func() any { return &backup.CreateInput{} }, Out: func() any { return &backup.CreateOutput{} }},
	},
	contracts.CapSourceWatch: {
		"poll":  {In: func() any { return &sourcewatch.PollInput{} }, Out: func() any { return &sourcewatch.PollOutput{} }},
		"dirty": {In: func() any { return &sourcewatch.DirtyInput{} }, Out: func() any { return new(bool) }},
		"close": {In: func() any { return &sourcewatch.CloseInput{} }, Out: func() any { return new(bool) }},
	},
}

// resolveOp returns the table entry for (cap, op). An empty op resolves
// only on single-input capabilities.
func resolveOp(cap, op string) (opSpec, error) {
	table, ok := ops[cap]
	if !ok {
		return opSpec{}, fmt.Errorf("unknown capability %s", cap)
	}
	if op == "" {
		if len(table) != 1 {
			return opSpec{}, fmt.Errorf("capability %s requires an op", cap)
		}
		for _, s := range table {
			return s, nil
		}
	}
	s, ok := table[op]
	if !ok {
		return opSpec{}, fmt.Errorf("capability %s has no op %s", cap, op)
	}
	return s, nil
}

// opForInput maps a concrete invoke input back to its op name. Nil
// input resolves to the op with no input body, or to the sole op when
// the capability has exactly one (an op with an optional input).
func opForInput(cap string, input any) (string, error) {
	table, ok := ops[cap]
	if !ok {
		return "", fmt.Errorf("unknown capability %s", cap)
	}
	for op, s := range table {
		if input == nil && s.In == nil {
			return op, nil
		}
		if input == nil || s.In == nil {
			continue
		}
		if reflect.TypeOf(input) == reflect.TypeOf(s.In()).Elem() {
			return op, nil
		}
	}
	if input == nil && len(table) == 1 {
		for op := range table {
			return op, nil
		}
	}
	return "", fmt.Errorf("capability %s has no op for input type %T", cap, input)
}
