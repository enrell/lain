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
docs/                      ARCHITECTURE, CONTRACTS, PLUGIN, RECOVERY
```

## API map

```text
GET  /health  /api/health
GET  /api/setup/status      POST /api/setup
POST /api/auth/login        GET  /api/me
GET  /api/libraries         POST /api/libraries
POST /api/library/scan      GET  /api/library/scan
GET  /api/catalog           GET  /api/catalog/{id}
GET  /api/search?q=&kind=
GET  /api/items/{id}/playback?client=&network=
GET  /api/items/{id}/stream            (Range, ?token= ok)
PUT  /api/items/{id}/progress          GET /api/items/{id}/progress
GET  /api/plugins           POST /api/plugins/swap
```

## Non-goals for v0.1

Transcoding, remote metadata providers, SQLite catalog, Matrix-daemon
mode, UI extensions, marketplace. Each is a plugin or seam away; the
contracts they will implement are already named in `docs/CONTRACTS.md`.
