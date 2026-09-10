# Lain

A local-first media server in Go, organized as replaceable plugins over a
small trusted core. Fast to build (stdlib + one small dep, seconds to
compile), local by default, swappable by design.

> Your data. Your machine. Your rules. Your plugins.

Status: **v0.1.0-dev**. Embedded core, file-backed catalog/userstate,
direct-play streaming, runtime plugin swap with generation fencing and
last-good fallback. See `docs/` for boundaries and contracts.

## Quickstart

```sh
go build -o lain ./cmd/lain
./lain serve --port 9360                        # data: ~/.local/share/lain
curl -X POST localhost:9360/api/setup -d '{"username":"admin","password":"password123"}'
TOK=$(curl -s -X POST localhost:9360/api/auth/login -d '{"username":"admin","password":"password123"}' | python3 -c "import sys,json;print(json.load(sys.stdin)['token'])")
curl -X POST localhost:9360/api/libraries -H "Authorization: Bearer $TOK" \
  -d '{"name":"Anime","type":"anime","path":"/media/anime"}'
curl -X POST localhost:9360/api/library/scan -H "Authorization: Bearer $TOK"
curl "localhost:9360/api/search?q=frieren" -H "Authorization: Bearer $TOK"
```

Stream with Range (mpv/desktop or media element, `?token=` fallback):

```sh
mpv "http://localhost:9360/api/items/<id>/stream?token=$TOK"
```

## Web UI

The server ships with a Svelte 5 SPA embedded in the binary — same
process, same origin, no Node at runtime:

```sh
make build          # pnpm build -> internal/webui/dist -> go build
./lain serve        # open http://127.0.0.1:9360
```

First run shows setup, then home/catalog/search/item/player/settings.
Playback honors the playback plan: mp4/webm/ogg direct-play in the
browser; other containers report honestly that no transcoder exists
(the desktop/CLI client plays them as-is). Progress is written in
bounded intervals, never on every video event.

Frontend development uses Vite HMR with the API proxied:

```sh
./lain serve              # terminal 1 (Go API)
cd web && pnpm dev        # terminal 2 (UI on :5173)
```

`go build ./...` does not need Node: a binary built without `make web`
serves a "web UI not built" notice on `/` while the API keeps working.

## Docker (Alpine, ~31 MB image, ~5 MiB idle)

```sh
UID=$(id -u) GID=$(id -g) docker compose up --build -d
```

Named volume `lain-data` holds the database; mount your media
read-only and register container paths (`/media/videos`) as libraries.
Memory is capped soft (`GOMEMLIMIT=256MiB`, GC-driven) and hard
(`mem_limit: 384m`). Healthcheck hits `/health`. The image builds the
frontend in a Node stage and embeds the static output in the Go
binary: the runtime image contains no Node, no frontend server and no
second origin.

## Watching (`lain watch`)
```sh
lain login --server http://127.0.0.1:9360 --username admin  # once; token in ~/.config/lain
lain watch frieren        # search, pick, play in mpv, save progress
lain watch --next         # resume first unfinished entry
lain watch frieren --once # no auto-next episode
lain watch silo --pick 2 --dry-run   # inspect without playing
```

`watch` resolves an item, launches mpv with an authenticated stream
URL, and reports progress through the same endpoint every client uses.
A bundled lua script (embedded in the binary) records position on
pause, every 10 s and on exit; completed episodes (≥95%) auto-play the
next one on a TTY. No credentials reach the player process.

## Backup & restore

```sh
lain backup --out backups              # online when the server runs, offline otherwise
lain restore backups/lain-backup-<ts> --data-dir ~/.local/share/lain
```

Online backup streams a consistent snapshot from one server read
transaction (safe mid-scan, mid-stream); offline needs the server
stopped and fails fast with a clear error instead of hanging on the
file lock. Restore validates structure first and refuses over a live
database — move it aside explicitly.

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
cmd/lain/                  CLI: serve, doctor, plugins, version
internal/contracts/        capability names + JSON shapes
internal/core/             composition, registry, generations, recovery
internal/store/            atomic JSON documents
internal/auth/             setup, bcrypt, HS256 tokens (stdlib JWT)
internal/gateway/          HTTP boundary (no plugin code touches net/http)
internal/matrix/           seam: manifests + doctor (embedded core in v0.1)
internal/plugins/source/   filesystem enumerator
internal/plugins/identify/ anime release parser + generic fallback
internal/plugins/catalog/  authoritative file catalog + export/import
internal/plugins/userstate/ progress, separate from catalog
internal/plugins/playback/ direct vs transcode-required planner
internal/plugins/search/   substring search (replaceable ranking)
internal/plugins/ingest/   scan orchestrator (policy-free pipeline)
internal/webui/            embedded SPA + static handler
web/                       SvelteKit source (Svelte 5, Tailwind 4)
docs/                      ARCHITECTURE, CONTRACTS, PLUGIN, RECOVERY
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
renamed to `*.imported`.

## Non-goals for v0.1

Transcoding, remote metadata providers, SQLite catalog, Matrix-daemon
mode, UI extensions, marketplace. Each is a plugin or seam away; the
contracts they will implement are already named in `docs/CONTRACTS.md`.
