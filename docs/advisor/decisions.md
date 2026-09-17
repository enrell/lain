# lain server decisions (citable in plans)

Use these IDs in plans and advisor calls: `D-001`, etc.
Update this file when the user answers a `needs-user` question.

## Stack and build

- `D-001` — Prefer the Go standard library. The only three justified dependencies are `golang.org/x/crypto` for bcrypt, `golang.org/x/term` for login, and `go.etcd.io/bbolt` for `lain.db`. A new dependency is a permanent cost and must be discussed first. Source: `AGENTS.md:6`.
- `D-002` — No cgo and no large C-transpiled dependencies. Builds must remain measured in seconds. Source: `AGENTS.md:20`.
- `D-003` — Use standard-library `net/http` method patterns without an HTTP framework. Source: `AGENTS.md:14`.
- `D-004` — Store data in bbolt through `internal/kv` using JSON values and composite keys in the `users`, `libraries`, `items`, `progress`, and `meta` buckets. Keep `internal/store` only for operator configuration and legacy import. Source: `AGENTS.md:15`.
- `D-005` — Use HS256 JWT in `internal/auth`, implemented with standard-library HMAC/SHA-256 and live role, liveness, and password-version checks. Source: `AGENTS.md:18`.

## Boundaries

- `D-006` — Plugins implement `core.Provider` and never touch `net/http`, process-global state, or another plugin's files. Source: `AGENTS.md:33`.
- `D-007` — Media bytes never enter plugin calls; the gateway streams from disk. Source: `AGENTS.md:35`.
- `D-008` — Catalog owns identity and userstate owns progress; never merge them. Source: `AGENTS.md:36`.
- `D-009` — The `merge-many` and `fan-out` modes belong to the metadata slice and must not be repurposed early. Transcode, metadata daemon, and Matrix daemon work begin only in their explicit slices. Source: `AGENTS.md:28`.

## Hygiene and discipline

- `D-010` — Never commit real release-group, fansub, tracker, indexer, or site names. Use fictional placeholders such as `[Fansub-A]`, `[Fansub-B]`, and `tracker-example`. Anime titles are allowed; group identities are not. Source: `AGENTS.md:42`.
- `D-011` — Use typed errors with stable `core.Error` codes; never panic on bad input and never fail silently. Source: `AGENTS.md:26`.
- `D-012` — Work in small slices and keep `go test ./...` green for behavior changes. Source: `AGENTS.md:24`.
- `D-013` — This repository is the server; the desktop client lives in `projects/lain-desktop`. Do not mix their commits. Source: `AGENTS.md:68`.
- `D-014` — English is the only canonical repository language for code, comments, tests, plans, project documentation, contributor instructions, and wikis in both repositories. Non-English content is limited to explicitly identified translations, localization resources, and language-specific documentation variants. Source: interview 2026-09-11.

## Product direction shared with lain-desktop

- `D-015` — Server evolution required by the desktop uses versioned API contracts, with implementation, tests, and commits kept separate in each repository. Source: interview 2026-09-11.
- `D-016` — Anime, movies, and series have product parity; the committed media scope is personal video, excluding music, audiobooks, photos, and live TV. Source: interview 2026-09-11.
- `D-017` — The Linux desktop may provision the server through environment-appropriate Docker, service, or binary methods using Polkit for privileged work, but may control only installations it provisioned. Source: interview 2026-09-11.
- `D-018` — After completing first use, stable identity and a correct collection model take priority over advanced playback, automation, or remote access. Source: interview 2026-09-11.
- `D-019` — Media identity v2 keeps catalog IDs stable across file moves and renames: rescans reconcile by logical fingerprint (library, kind, normalized title, season, episode, compatible year) and adopt the existing ID onto the new path with the previous path in `aliases`, guarded by the scan present-set so genuine duplicates never merge. Export format bumped to `lain.catalog-export` version 2 (import accepts 1 and 2); `ScanStats` gains `migrated`. Source: user answer QQ-S4-03, 2026-09-11.
- `D-020` — Keep the web UI in Svelte. It follows the live Omarchy palette from the server host through a public, read-only endpoint that returns only validated semantic colors and mode; all browsers share that palette, and deployments without Omarchy retain the built-in fallback. Source: user answers Q-001 and Q-002, 2026-09-11.
- `D-021` — Before selecting the next WebUI direction, create an isolated static showroom under `web/design-showcase/` with ten distinct, non-binding HTML concepts and one comparison index. Concepts may explore visual language, information architecture, navigation, hierarchy, and interaction, but must not enter production routes or the build and must add no dependency. Source: user answers Q-003 and Q-004, 2026-09-11.
- `D-022` — Redesign the production Svelte WebUI around showroom Concept 04 Monolith: a clean top navigation and title-first cinematic hero, followed by Concept 01-style useful media rails. Preserve the current hero selection policy (resume first, otherwise recently added), Continue Watching landscape cards, Recently Added, and one rail per library. Use restrained cinematic motion with reduced-motion support; preserve search, account, server status, mobile, authentication, and existing behaviors. Source: design interview and final user confirmation, 2026-09-11.

- `D-037` — The legibility floor for operator-palette roles lives in the server derivation, not in the components: `internal/gateway/theme.go` derives distinct, contrast-checked values per semantic role (`muted` clears 4.5:1 against the background; `line`, `surface-active` and the text they carry are never identical), so a host Omarchy theme that collapses several roles onto one swatch cannot render unreadable text. The endpoint shape, the host as the palette source, the public read-only route and the built-in fallback all stay as D-020 fixed them; this bounds the derivation, it does not change the contract. Source: user answer Q-025, 2026-09-17.

- `D-038` — The player's preparation overlay shows an honest, bounded wait: elapsed time derived from the server's already-public `started_at`/`queued_at` (a reload must not restart the story), copy naming the first play and the multi-minute cost of sources that need a full re-encode (AV1/HEVC), an explicit statement that leaving continues preparation server-side, and a leave/back affordance. Web-only: no dependency, no server change, no contract change. Source: user answer Q-026/A, 2026-09-17.

## Transcode slice

- `D-023` — Begin the reserved transcode slice now, server-only: ffmpeg as an external binary (`LookPath` + degrade when absent, no Go dependency, no cgo), first slice bounded to browser MKV→compatible-MP4. Source: user answers Q-007 and Q-009, 2026-09-12.
- `D-024` — Transcode ships as an additive versioned contract change: new Plan mode/session, new gateway stream endpoint with Range support, `lain.playback.transcode@1` provider. Source: user answer Q-008, 2026-09-12.

## Metadata providers

- `D-025` — TVMaze-only slice: add keyless TVMaze series provider (stdlib HTTPS via `politeClient`, last in merge-many precedence so anime behavior is unchanged, composition v2 migration appends it to saved v1 bindings). No TMDB (no user key supplied), no IMDb scraper (no free official API; IMDb IDs arrive only as TVMaze externals), contract untouched until the Q-012 field list is specified. Source: advisor triage + user answers Q-010/B, Q-011/A, Q-013/A, 2026-09-12.

## Observability

- `D-026` — Slice 1: stdlib `log/slog` to stdout (no dependency, no cgo), `--log-level` flag + `LAIN_LOG_LEVEL` (debug/info/warn/error, default info), per-request access lines with server-side request ids, enrich/ffmpeg/scan diagnostics; no new endpoint, no file sink. Absolute ban on logging secrets (tokens, passwords, keys) and media bytes. File sink (Q-014), `X-Request-ID` header (Q-015) and enrich error `detail` (Q-016) stay out until answered. Source: user request + advisor triage, 2026-09-12.
- `D-027` — Logs are machine-first for AI agents, imitating the open OpenTelemetry Logs Data Model with stdlib only (no SDK): `Timestamp`, `SeverityText`, `SeverityNumber` (5/9/13/17), `Body`, `Resource` (`service.name`, `service.version`), flat event Attributes, `req` correlation id; `TraceId`/`SpanId` reserved. Dev container runs full log (`LAIN_LOG_LEVEL=debug` in `docker-compose.dev.yml`). Schema documented in `docs/LOGGING.md`. Source: user direction 2026-09-12 + OTel spec (external fact, wiki-uncontradicted).

## Transcode evolution

- `D-028` — Add asynchronous transcoding as `lain.playback.transcode@2` while retaining the synchronous `@1` contract and GET endpoint. Playback inspection is side-effect free and reports an opaque session plus `idle|queued|running|ready|failed`; only an authenticated POST starts or joins work, and an item-scoped status endpoint exposes state without filesystem paths. Source: user answer Q-017/A + advisor triage, 2026-09-13.
- `D-029` — Bound ready transcode artifacts with a plugin-private 20 GiB LRU cache, configurable by `--transcode-cache-size` / `LAIN_TRANSCODE_CACHE_SIZE`. Metadata uses atomic sidecars; cleanup runs at startup and around admission/completion, excludes active artifacts, and removes stale-source and abandoned-temporary entries without coupling the plugin to library deletion. Source: user answer Q-018/A + delegated engineering choice + advisor triage, 2026-09-13.
- `D-030` — Browser compatibility becomes probe-driven and applies equally to anime, movies and series: direct-play only verified web-safe streams; otherwise produce one conservative SDR H.264/AAC MP4, with selected stream identity in the session/cache key. Add selectable audio first, then convertible text subtitles as WebVTT sidecars; reject HDR and unsupported subtitle tracks explicitly until separately scoped tone-map/burn-in profiles. Source: user-delegated engineering choice Q-019 + advisor triage, 2026-09-13.
- `D-031` — Keep progressive MP4 with HTTP Range as the delivery protocol. HLS/DASH, ABR ladders and hardware acceleration stay deferred; future hardware support must be explicit operator opt-in with capability probing and visible software fallback, never silent opportunistic switching. Source: user-delegated engineering choice Q-020 + advisor triage, 2026-09-13.

- `D-039` — `lain.playback.transcode@2` gains one additive status field, `progress` (0..1, omitempty), present while a job runs; no version bump and no server-side ETA promise — the client derives elapsed time and pace from deltas. The field is additive but the @2 payload had been fixed by D-028, so it ships only under this explicit answer. Source: user answer Q-027/A, 2026-09-17.
- `D-040` — The AV1 wait is not paid by weakening the frozen profile: `web-mp4-sdr-v2` / `veryfast` / software-only stays exactly as D-030/D-031 fixed it. The wait is instead reserved to its own slice, authorized but not started: browser-capability direct play, where the client reports decoder support and a verified source remuxes (AV1/Opus into MP4 plus a WebVTT sidecar) instead of re-encoding. That slice needs its own plan before any code. Source: user answer Q-028/C, 2026-09-17.
- `D-041` — Preparation stays play-triggered: only the player's authenticated POST starts work (D-028 intact); no details-page CTA and no proactive warm after scan. Source: user answer Q-029/A, 2026-09-17.

## Metadata contract freeze

- `D-032` — Freeze `MetadataRecord`/`Enrichment` on existing poster/cover/synopsis/genres fields: no `native_title` additive field (Q-006/B), Monolith hero ships without the JP line, and no broader extension until a new field list with version bump is specified (Q-012/A). Source: user answers 2026-09-15 + advisor triage.

## Observability contract confirmation

- `D-033` — Confirm D-026 slice 1 boundaries: stdout-only logs, server-side request-id correlation with no `X-Request-ID` header (Q-015/B), and byte-identical admin enrich errors with per-provider causes kept in server logs only and no additive `detail` (Q-016/A). Source: user answers 2026-09-15 + advisor triage.

## QA agent fleet

- `D-034` — The fleet drives a real Chromium over the DevTools Protocol through the repo-owned `qa/browser.mjs` (Node built-ins only, no dependency, no cgo, D-001/D-002 untouched); OpenCode's integrated `browser.*` tools are used only when a desktop client is attached, chosen by `LAIN_AGENT_BROWSER=auto|cdp|integrated` with a probe that proves the backend before spending tokens. Evidence: headless `opencode2 run --agent lain-qa-probe` reports the integrated tools as unavailable. Source: user request for agents with browser access + advisor triage, 2026-09-16.
- `D-035` — QA runs against a disposable instance by default (own bbolt dir, synthetic fixtures from `web/e2e/smoke.mjs`'s generator, free port, torn down after); auditing an already-running server requires an explicit `LAIN_AGENT_TARGET_URL`, prints a loud banner naming what will be mutated, waits, and refuses to start without real admin credentials. Reports, screenshots and ledgers stay local under gitignored `qa/runs/`. An interrupted or recovered audit is recorded as a coverage gap and can never read as a clean run. A finding whose fix would break a frozen contract is parked as `needs-decision` for the user, not settled as `wont-fix`. Source: user answers "Sempre local" + cyclic fix/verify direction, advisor triage 2026-09-16.
- `D-036` — Fleet shape: specialist prompts are tracked in `qa/agents/*.md` and installed flat into the ignored `.opencode/agents/` by `just agent-sync`; operational contract lives in generated briefs (`qa/prompts/*.md`) so harness changes never rewrite a specialisation. Model and provider come from `.env` (`LAIN_AGENT_MODEL`) with no built-in fallback and no credential in the repo (OpenCode's own auth store). Agent commits go to a dedicated `agent/qa/<run>` branch, never `main`, never pushed, and the runner reverts a round whose `just check` goes red. Source: user direction 2026-09-15/16 + advisor triage.

