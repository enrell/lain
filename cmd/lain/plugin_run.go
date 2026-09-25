package main

import (
	"fmt"
	"path/filepath"

	"github.com/enrell/lain/internal/component"
	"github.com/enrell/lain/internal/core"
	"github.com/enrell/lain/internal/plugins/identify"
	"github.com/enrell/lain/internal/plugins/metadata"
	"github.com/enrell/lain/internal/plugins/playback"
	"github.com/enrell/lain/internal/plugins/probe"
	"github.com/enrell/lain/internal/plugins/source"
	"github.com/enrell/lain/internal/plugins/thumbnail"
	"github.com/enrell/lain/internal/plugins/transcode"
)

// cmdPluginRun serves one built-in provider as a component over the
// component protocol (D-074): `lain plugin-run --id <id> --sock <path>`.
// A manifest in <data-dir>/plugins/ whose entrypoint is the lain binary
// with args ["plugin-run","--id","<id>","--sock","{sock}"] runs that
// built-in out-of-process.
func cmdPluginRun(args []string) error {
	id := flag(args, "id", "")
	sock := flag(args, "sock", "")
	dataDir := flag(args, "data-dir", defaultDataDir())
	if id == "" || sock == "" {
		return fmt.Errorf("plugin-run needs --id and --sock")
	}
	p, err := componentProvider(id, dataDir)
	if err != nil {
		return err
	}
	return component.Serve(sock, p)
}

// componentProvider builds the built-ins that are self-contained: they
// need no shared db handle and no registry. Catalog, userstate, ingest,
// search and enrich stay in-process by design — they own or compose
// trusted-core state.
func componentProvider(id, dataDir string) (core.Provider, error) {
	switch id {
	case "lain-source-filesystem":
		return source.Provider{}, nil
	case "lain-identify-anime":
		return identify.Anime{}, nil
	case "lain-identify-generic":
		return identify.Generic{}, nil
	case "lain-probe-ffprobe":
		return probe.Provider{}, nil
	case "lain-playback-default":
		return playback.Planner{}, nil
	case "lain-thumbnail-ffmpeg":
		return thumbnail.New(filepath.Join(dataDir, "thumbnails")), nil
	case "lain-transcode-ffmpeg":
		return transcode.NewConfigured(filepath.Join(dataDir, "transcodes"), transcode.Config{}), nil
	case "lain-metadata-nfo":
		return metadata.NFO{}, nil
	case "lain-metadata-kitsu":
		return metadata.NewKitsu(), nil
	case "lain-metadata-anilist":
		return metadata.NewAniList(), nil
	case "lain-metadata-jikan":
		return metadata.NewJikan(), nil
	case "lain-metadata-tvmaze":
		return metadata.NewTVMaze(), nil
	default:
		return nil, fmt.Errorf("provider %q cannot run as a component (needs shared db or registry)", id)
	}
}
