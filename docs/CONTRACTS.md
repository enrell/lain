# Contracts

Versioned capability names. A name is not a credential; serving rights
come from the composition.

## lain.source.enumerate@1 (exactly-one)

Input: `{root, library_id, type}` → Output: `Candidate[]`.

Discovers files only. Walk errors are counted, never fatal to a scan.

## lain.media.identify@1 (ordered-many)

Input: `Candidate` → Output: `Proposal{kind,title,season,episode,year,
confidence,evidence,plugin_id}`.

First `Accepted()` proposal (non-empty title, confidence > 0) wins.
Built-ins: `lain-identify-anime` (declines without release evidence),
`lain-identify-generic` (always accepts, confidence 0.4).

## lain.catalog.read@1 / lain.catalog.write@1 (exactly-one)

Write input: `{library_id, proposal, candidate}` → `CatalogItem` with
`origin` + `provenance`. Item id is stable over library+path. Read:
`nil` → list, `{id}` → one item.

Portable document `lain.catalog-export@1` is the replacement contract.

## lain.userstate.progress@1 (exactly-one)

`PutInput{user_id, progress}` / `GetInput{user_id, item_id}`. Keyed by
user+item in its own document; catalog rewrites never touch it.

## lain.playback.plan@1 (first-accepted)

Input: `{request{item_id, client, network}, file_path}` → `Plan{mode,
asset, available, reason?}`. `asset` is opaque (`asset:<id>`); the
gateway resolves it. mpv/desktop clients always direct-play; browser
clients facing non-web containers get `transcode-required` with
`available:false` until a transcode provider exists.

## lain.search.query@1 (exactly-one)

Input: `{q, kind}` → `CatalogItem[]`. v0.1 is case-insensitive substring
over titles; ranking policy is the replaceable unit.

## lain.ingest.scan@1 (exactly-one)

Input: `{libraries[]}` → `ScanStats{libraries, candidates, identified,
unidentified, errors, ...}`. Fixed order: enumerate → identify →
catalog write. Unidentified files count, never abort.

## Declared (next slice)

`lain.metadata.search@1` / `resolve@1` / `artwork@1` (merge-many),
`lain.playback.transcode@1`, `lain.transform.*@1`, `lain.sync.*@1`.
Names are reserved here so first implementers do not collide.
