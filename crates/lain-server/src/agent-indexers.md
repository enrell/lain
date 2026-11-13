# lain: indexers API for coding agents

Format: lain.agent-guide@1 (D-291). Served at `GET /api/agent/indexers.md`
by every lain server; no key needed to read it.

You were given a lain server URL and an integration key (`lain_…`) with the
`indexers` scope. With it you can list, add, test, change and remove the
server's Torznab/Newznab indexers, and nothing else: every other route
answers 401. The key expires (the owner chose when, one hour by default) and
the owner can revoke it at any time; then every call answers 401 and you
stop.

## Rules

1. Ask the owner before you add, change or remove anything. Show what you
   found (names, URLs without keys, what a probe answered) and wait for a
   yes. Reading (list, discover, probe, test) needs no permission.
2. Send the key only in the header `Authorization: Bearer <key>`. Never put
   it in a URL, a file, a commit or your output; lain refuses a key in
   `?token=`.
3. Never print an indexer's or aggregator's API key back. lain never returns
   stored keys (`has_api_key: true` instead).
4. Do not invent indexer URLs. Use what the owner gives you or what
   `discover` returns.

## Calls

`$LAIN` is the server URL, `$KEY` the key. Bodies are JSON
(`Content-Type: application/json`). Errors are `{"error": "...", "code": "..."}`
with a 4xx/5xx status.

```sh
H="Authorization: Bearer $KEY"

# List the indexers lain has (keys never shown).
curl -s -H "$H" "$LAIN/api/acquire/indexers"

# Ask a Prowlarr (base URL like http://127.0.0.1:9696) or a Jackett
# (http://127.0.0.1:9117) which indexers it serves. source: prowlarr | jackett.
# Each answer row: source_id, name, protocol, url (the feed), privacy,
# enabled, added_as (the lain indexer already on that feed, if any).
curl -s -H "$H" -H 'Content-Type: application/json' -X POST "$LAIN/api/acquire/indexers/discover" \
  -d '{"source": "prowlarr", "url": "http://127.0.0.1:9696", "api_key": "<prowlarr key>"}'

# Add the chosen ones in one step (after the owner said yes). lain asks the
# aggregator again and stores its key on each; ones already there are
# skipped. 201 {"added": [...], "skipped": ["<source_id>"]}.
curl -s -H "$H" -H 'Content-Type: application/json' -X POST "$LAIN/api/acquire/indexers/import" \
  -d '{"source": "prowlarr", "url": "http://127.0.0.1:9696", "api_key": "<prowlarr key>", "ids": ["1", "4"]}'

# Try a single Torznab/Newznab feed without saving it: caps and a sample of
# its latest releases. 200 {"ok": bool, "caps": {...}, "sample": [...], "error"?}.
# With "id" it tries a saved indexer with your fields laid over it.
curl -s -H "$H" -H 'Content-Type: application/json' -X POST "$LAIN/api/acquire/indexers/probe" \
  -d '{"protocol": "torznab", "url": "https://example.invalid/api", "api_key": "<indexer key>"}'

# Add one feed (after the owner said yes). protocol: torznab | newznab.
# Optional: categories [int], enabled (default true), priority,
# min_interval_ms (default 2000; the gap between two requests).
curl -s -H "$H" -H 'Content-Type: application/json' -X POST "$LAIN/api/acquire/indexers" \
  -d '{"name": "my-indexer", "protocol": "torznab", "url": "https://example.invalid/api", "api_key": "<indexer key>"}'

# Test a saved indexer (caps); records its health. 200 {"ok": bool, "error"?}.
curl -s -H "$H" -X POST "$LAIN/api/acquire/indexers/<id>/test"

# Change one; absent fields stay, "api_key": "" keeps the stored key.
curl -s -H "$H" -H 'Content-Type: application/json' -X PATCH "$LAIN/api/acquire/indexers/<id>" \
  -d '{"enabled": false}'

# Remove one (after the owner said yes).
curl -s -H "$H" -X DELETE "$LAIN/api/acquire/indexers/<id>"
```

## A good session

1. `GET /api/acquire/indexers` to see what is there.
2. Ask the owner where their indexers come from: a Prowlarr, a Jackett, or
   feed URLs they have. Ask for the base URL and API key if needed.
3. `discover` (or `probe` for a single feed) and show the owner the list.
4. On a yes, `import` the chosen ids (or `POST` each feed), then `test` each
   new indexer and report which answered.
5. Tell the owner they can revoke the key now in Settings › Indexers.

The full contract is in docs/CONTRACTS.md of the lain repository
(`/api/acquire/indexers*`, "Integration keys").
