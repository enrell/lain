# Lain

[![CI](https://github.com/enrell/lain/actions/workflows/ci.yml/badge.svg)](https://github.com/enrell/lain/actions/workflows/ci.yml)
[![Release](https://github.com/enrell/lain/actions/workflows/release.yml/badge.svg)](https://github.com/enrell/lain/releases)
[![Container](https://img.shields.io/badge/ghcr.io-enrell%2Flain-2496ED?logo=docker&logoColor=white)](https://github.com/enrell/lain/pkgs/container/lain)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)

A local-first media server in Go, organized as replaceable plugins over a
small trusted core. Fast to build (stdlib + one small dep, seconds to
compile), local by default, swappable by design.

> Your data. Your machine. Your rules. Your plugins.

**v0.1.0** ships as: a single static binary with the web UI embedded, a
multi-arch container image, and a desktop client
([lain-desktop](https://github.com/enrell/lain-desktop)). The core is
file-backed and dependency-light; the plugin registry supports runtime
swap with generation fencing and last-good fallback. See `docs/` for
boundaries and contracts.

## Install

### Docker (recommended, no toolchain needed)

```sh
docker run -d --name lain \
  -p 9360:9360 \
  -v lain-data:/data \
  -v ~/Videos:/media/videos:ro \
  ghcr.io/enrell/lain:latest
```

Open <http://127.0.0.1:9360>, create the admin account, add a library
pointing at `/media/videos`, and scan. The same image runs on
`linux/amd64` and `linux/arm64`.

Prefer compose? Start from the shipped [`docker-compose.yml`](docker-compose.yml)
or let the installer generate a filled-in one (data dir, media dirs,
port, tag). The image bundles `ffmpeg` for thumbnails and
`ca-certificates` for remote metadata providers; no Node, no second
origin — the SPA is embedded in the binary.

### Interactive Linux installer

One script covers server and desktop in all combinations (server +
desktop, server only, desktop only; Docker, static binary or user
daemon; media paths, port, compose generation):

```sh
curl -fsSL https://raw.githubusercontent.com/enrell/lain/main/scripts/install.sh -o lain-install.sh
less lain-install.sh          # read it first, it is yours to run
bash lain-install.sh
```

### Static binary

```sh
# from a release (linux/amd64 or linux/arm64):
curl -fsSLO https://github.com/enrell/lain/releases/latest/download/lain_0.1.0_linux_amd64.tar.gz
curl -fsSLO https://github.com/enrell/lain/releases/latest/download/lain_0.1.0_linux_amd64.tar.gz.sha256
sha256sum -c lain_0.1.0_linux_amd64.tar.gz.sha256
tar -xzf lain_0.1.0_linux_amd64.tar.gz
install -Dm755 lain ~/.local/bin/lain
lain serve --port 9360          # data: ~/.local/share/lain
```

### From source

```sh
git clone https://github.com/enrell/lain && cd lain
make build            # pnpm build -> internal/webui/dist -> go build
./lain serve --port 9360
```

`go build ./...` does not need Node: a binary built without `make web`
serves a "web UI not built" notice on `/` while the API keeps working.

## First steps

```sh
curl -X POST localhost:9360/api/setup -d '{"username":"admin","password":"password123"}'
TOK=$(curl -s -X POST localhost:9360/api/auth/login -d '{"username":"admin","password":"password123"}' \
  | python3 -c "import sys,json;print(json.load(sys.stdin)['token'])")
curl -X POST localhost:9360/api/libraries -H "Authorization: Bearer $TOK" \
  -d '{"name":"Anime","type":"anime","path":"/media/anime"}'
curl -X POST localhost:9360/api/library/scan -H "Authorization: Bearer $TOK"
curl "localhost:9360/api/search?q=frieren" -H "Authorization: Bearer $TOK"
```

Stream with Range (mpv/desktop or media element, `?token=` fallback):

```sh
mpv "http://localhost:9360/api/items/<id>/stream?token=$TOK"
```

## Features

- **Plugins over a tiny core.** Every capability (source, identify,
  catalog, userstate, playback, search, ingest, metadata, thumbnails)
  is bound in a composition; `POST /api/plugins/swap` replaces the
  provider at runtime with generation fencing and last-good fallback.
- **Direct play, honest plans.** `GET /api/items/{id}/playback` answers
  `direct` or `transcode-required`; with no transcoder installed it says
  so instead of faking a stream. mpv-based clients direct-play mkv/hevc.
- **Metadata enrichment.** Local NFO sidecars plus Kitsu/AniList/Jikan
  (merge-many, scored, TTL cached). Overlays decorate the catalog
  without touching identity, files or progress; grids read them in one
  batch (`GET /api/enrichments?ids=`).
- **Thumbnails.** `GET /api/items/{id}/thumbnail?t=&w=` extracts a JPEG
  with ffmpeg (capability `lain.transform.thumbnail@1`), caches it on
  disk and serves it with daily cache headers. No ffmpeg? Only this
  capability degrades.
- **Progress that survives reindexing.** User state lives outside the
  catalog: continue watching, resume, bounded writes, per-user.
- **Multi-user roles.** Reads and watching for everyone; libraries,
  scans, users, plugins and backups are admin-only. Disabling or
  rotating a password kills live tokens on the next request.
- **Backup/restore.** Online backups stream a consistent snapshot from
  one read transaction (safe mid-scan, mid-stream); restore validates
  structure and refuses over a live database.
- **Embedded web UI.** Svelte 5 SPA compiled into the Go binary — same
  process, same origin, no Node at runtime.

## Desktop client

A native client lives in [enrell/lain-desktop](https://github.com/enrell/lain-desktop):
Qt 6 Quick/QML + libmpv, real API integration (first-run setup, login,
catalog, search, enrichment, playback with resume), server-side
thumbnails and a headless test suite. The installer above can set up
both. Releases provide a Linux tarball; on Arch a source build is one
`pacman` away.

## CLI

```sh
lain serve --port 9360                  # data: ~/.local/share/lain
lain doctor                             # environment report
lain plugins                            # registered providers + composition
lain version
lain login --server http://127.0.0.1:9360 --username admin
lain watch frieren                      # search, pick, play in mpv, save progress
lain watch --next                       # resume first unfinished entry
lain backup --out backups               # online when the server runs
lain restore backups/lain-backup-<ts> --data-dir ~/.local/share/lain
```

`watch` resolves an item, launches mpv with an authenticated stream URL
and reports progress through the same endpoint every client uses. A
bundled lua script records position on pause, every 10 s and on exit;
completed episodes (≥95%) auto-play the next one on a TTY. No
credentials reach the player process.

## Web UI development

```sh
./lain serve              # terminal 1 (Go API)
cd web && pnpm dev        # terminal 2 (Vite HMR on :5173, API proxied)
```

## Runtime plugin swap (the point of the project)

```sh
# Replace the anime identifier with the generic fallback, no restart:
curl -X POST localhost:9360/api/plugins/swap -H "Authorization: Bearer $TOK" \
  -d '{"capability":"lain.media.identify@1","providers":["lain-identify-generic"],"generation":1}'
# A stale writer is fenced out:
# {"code":"stale-generation","error":"expected generation 1, active is 2",...}
```

A swap health-checks candidates first: an unhealthy provider is rejected
and the previous generation keeps serving. A provider that fails at call
time falls back to the last-good one, once, with the incident logged.

## Layout

```text
cmd/lain/                   CLI: serve, doctor, plugins, watch, backup, version
internal/contracts/         capability names + JSON shapes
internal/core/              composition, registry, generations, recovery
internal/store/             atomic JSON documents
internal/auth/              setup, bcrypt, HS256 tokens (stdlib JWT)
internal/gateway/           HTTP boundary (no plugin code touches net/http)
internal/matrix/            seam: manifests + doctor (embedded core in v0.1)
internal/plugins/source/    filesystem enumerator
internal/plugins/identify/  anime release parser + generic fallback
internal/plugins/catalog/   authoritative file catalog + export/import
internal/plugins/userstate/ progress, separate from catalog
internal/plugins/playback/  direct vs transcode-required planner
internal/plugins/search/    substring search (replaceable ranking)
internal/plugins/ingest/    scan orchestrator (policy-free pipeline)
internal/plugins/metadata/  NFO + Kitsu/AniList/Jikan, merge-many + cache
internal/plugins/thumbnail/ ffmpeg stills, on-disk cache, path not bytes
internal/webui/             embedded SPA + static handler
web/                        SvelteKit source (Svelte 5, Tailwind 4)
scripts/install.sh          interactive Linux installer
site/                       project site (GitHub Pages)
docs/                       ARCHITECTURE, CONTRACTS, PLUGIN, RECOVERY
```

## API map

```text
GET  /health  /api/health
GET  /api/setup/status      POST /api/setup          (first admin)
POST /api/auth/login        GET  /api/me
PATCH /api/me/password      GET  /api/me/continue
GET  /api/users             POST /api/users          (admin)
PATCH /api/users/{id}                                (admin: disable/role/reset)
GET  /api/libraries         POST /api/libraries      (admin)
DELETE /api/libraries/{id}                           (admin)
POST /api/library/scan      GET  /api/library/scan  (start: admin)
GET  /api/catalog?limit=&offset=&sort=&library_id=  (envelope {items,total}; sort=title|recent)
GET  /api/catalog/{id}
GET  /api/search?q=&kind=&limit=&offset=&sort=   (same envelope)
GET  /api/items/{id}/playback?client=&network=
GET  /api/items/{id}/stream            (Range, ?token= ok)
GET  /api/items/{id}/thumbnail?t=&w=   (JPEG still; ?token= ok, cached on disk)
PUT  /api/items/{id}/progress          GET /api/items/{id}/progress
POST /api/catalog/{id}/enrich          (admin; NFO/Kitsu/AniList/Jikan merge)
GET  /api/catalog/{id}/enrich
GET  /api/enrichments?ids=a,b,c        (batch overlay read, max 200)
DELETE /api/catalog/{id}/enrich        (admin)
GET  /api/plugins           POST /api/plugins/swap  (admin)
```

## Users & storage

Multi-user with roles (`admin`, `user`). Reads and watching are for
everyone; libraries, scans, plugins and user administration are
admin-only. Disabling and password rotation kill live tokens on next
request (password version rides the JWT and is checked live).

State lives in one embedded database (`lain.db`, bbolt — pure Go, no
cgo, millisecond builds intact): users, libraries, catalog, progress.
A v0.1 JSON data dir is imported once on first boot and its files
renamed to `*.imported`. Thumbnails cache under `<data>/thumbnails/`.

## Docker notes

The application image is Alpine-based and built for `linux/amd64` and
`linux/arm64`. It includes `ffmpeg` (thumbnails) and
`ca-certificates` (remote metadata over TLS), so it is around 210 MB on
disk — the alternative is a server whose thumbnail capability reports
itself unavailable.

It runs as the unprivileged `lain` user with `/data` as the volume
(`LAIN_DATA_DIR=/data`), binds `0.0.0.0:9360` and includes a
`HEALTHCHECK`. Named volumes are initialized with the right ownership;
media should be mounted read-only and be readable by the container user
(default 0644/0755 umask is fine). If you bind-mount a host data
directory instead, make it writable by uid 1000 or run the container
with `--user`.

Compose defaults cap memory softly (`GOMEMLIMIT=256MiB`) and hard
(`mem_limit: 384m`); thumbnail extraction runs inside the same cgroup,
so raise both for very large libraries or high-resolution stills.

## Testing

```sh
go test ./...                 # unit + gateway e2e (thumbnail tests skip without ffmpeg)
go vet ./...
make web                      # frontend build + embed
web/e2e/smoke.mjs             # Chromium/CDP end-to-end against a fresh data dir
```

The e2e smoke covers setup, libraries, scan, browse, search, NFO
enrichment, thumbnails, playback (Range, keyboard seek, bounded
progress writes), Continue Watching resume, users, composition swap
with generation fencing, backup download, non-admin gating, deep-link
refresh, responsive layouts and a console/network quality gate.

## Releases

Tags `vX.Y.Z` publish:

- container images `ghcr.io/enrell/lain:{X.Y.Z, X.Y, latest}` (multi-arch);
- `lain_X.Y.Z_linux_{amd64,arm64}.tar.gz` binaries with `.sha256`
  sidecars and a combined `checksums.txt`;
- the matching [desktop release](https://github.com/enrell/lain-desktop/releases)
  is versioned independently.

## License

Apache-2.0. See [LICENSE](LICENSE).
