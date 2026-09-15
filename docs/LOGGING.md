# Logging — agent field guide

Machine-first JSONL on stdout: one OpenTelemetry Logs Data
Model-shaped record per line, stdlib only, no SDK. Humans can read
it with `jq`; agents parse every line with one stable schema
(D-026, D-027).

```json
{"Timestamp":"2026-09-12T05:00:02.654Z","SeverityText":"WARN","SeverityNumber":13,"Body":"enrich no match","Resource":{"service.name":"lain","service.version":"0.1.0-dev"},"req":"req-21","item":"4176127ae73b2eda","title":"Whatever"}
```

## Record schema

| Field | Source | Notes |
|---|---|---|
| `Timestamp` | OTel `Timestamp` | RFC 3339 UTC, event time |
| `SeverityText` | OTel `SeverityText` | `DEBUG`, `INFO`, `WARN`, `ERROR` |
| `SeverityNumber` | OTel `SeverityNumber` | 5 / 9 / 13 / 17 (spec bands) |
| `Body` | OTel `Body` | Short snake-case event name (`request`, `enrich ok`, `scan done`) |
| `Resource` | OTel `Resource` | `service.name` (`lain`), `service.version` |
| `req` | correlation id | Server-side request id; every line of one HTTP request shares it. Wire header reserved (Q-015) |
| remaining keys | OTel `Attributes` | Event context: `method`, `path`, `status`, `dur_ms`, `item`, `title`, `provider`, `provider` skip `causes`, scan counters, `err` |

`TraceId`/`SpanId` are reserved for a future tracing slice.

## Levels

`--log-level` / `LAIN_LOG_LEVEL`: `debug`, `info` (default),
`warn`, `error`. Unknown values refuse to boot (exit 1).
The dev container (`docker-compose.dev.yml`) runs full log
(`debug`); production stays at `info` unless overridden.

| Level | What lives here |
|---|---|
| `DEBUG` | every request line, metadata search outcomes, per-provider skip causes, enrich winners |
| `INFO` | boot, log level, scan start/done with counters |
| `WARN` | 4xx responses, enrich search/resolve misses, thumbnail/transcode failures, failed logins |
| `ERROR` | 5xx responses, scan failures, enrich save failures, bad provider results |

## Recipes

```sh
just logs                                  # follow the dev API stream
just logs 2>&1 | grep '"SeverityNumber":1[37]'   # warnings and errors only
echo "$line" | jq .                        # pretty-print one record
# one request, all its lines:
grep '"req":"req-21"' api.log | jq -c '{Timestamp,Body,status,item,err}'
# why is a show unenriched:
grep 'enrich' api.log | jq -c 'select(.item=="<id>") | {Timestamp,Body,provider,err}'
# which upstream is down:
grep 'providers skipped' api.log | jq -r .causes
```

## Secret policy (absolute)

Query strings, request/response bodies and media bytes never
enter a line: media `?token=` rides in the query, login carries
passwords. Handlers log ids, paths, counts and error causes.
A test (`TestAccessLogHidesSecrets`) fails the build on leakage.
