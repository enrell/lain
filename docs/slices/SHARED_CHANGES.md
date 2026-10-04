# Shared changes

Edits a slice made outside its own files, so parallel slices can avoid
or resolve conflicts. One section per slice; one row per touched spot.

## Social (`feat/social`)

Everything else the slice adds is in new files: `internal/contracts/social.go`,
`internal/kv/social.go`, `internal/plugins/social/`, `internal/gateway/social.go`
(+ tests), `docs/slices/social.md`, and on the web `lib/api/social.ts`,
`lib/social/`, `lib/stores/social.svelte.ts`, `lib/components/social/`,
`routes/social/`, `routes/u/`, `routes/settings/privacy/`.

| File | Change | Conflict risk |
|---|---|---|
| `internal/core/composition.go` | +4 bindings (`lain.social.*@1` → `lain-social-bolt`) appended after `lain.settings.integrations@1`, behind a comment. **No `Version` bump** — `Upgrade` adds missing capabilities on its own, so the reading slice keeps the next version number. | Low: append-only inside the bindings map. |
| `internal/gateway/server.go` | import `plugins/social`; `social.New(db)` after `list.New`; `reg.Register(soc)` after `reg.Register(lst)`; `s.routesSocial()` after `s.routesList()`; one line in `handleProgressPut` after the scrobble block: `s.recordSocialProgress(v.UserID, prev, in)`. | Low: five one-line insertions next to list-slice lines. |
| `internal/kv` | **Not edited**: social buckets are declared in the new `internal/kv/social.go` and created by the social provider, so `kv.Open`'s bucket list is untouched. | none |
| `docs/CONTRACTS.md` | New `## Social` section inserted right before `## Component mode (wire protocol)`. | Low. |
| `docs/advisor/decisions.md` | **Not edited**: social choices are `S-*` in `docs/slices/social.md` until merge, so neither slice grabs the next `D-` number. | none |
| `web/src/lib/api/index.ts` | import + `social` entry in `api`; `export type * from './social'` and `targetOf`. Types live in the new `api/social.ts`; `types.ts` is **not** edited. | Low: one line in each list. |
| `web/src/lib/i18n/messages/en.ts` | `nav.social`, `nav.unread`; `settings.section.privacy`; six `settings.entry.*` (privacy rows); a new top-level `social` block appended before `} as const`. | Medium: the reading slice will likely add keys too — both append, so merges are mechanical. |
| `web/src/lib/settings/sections.ts` | `privacy` section (`/settings/privacy`, chord `g v`) after `security`; six palette entries after `password`. | Low. Chord `v` is now taken. |
| `web/src/lib/components/navigation/AppShell.svelte` | `/social` link after `/list`, unread badge on it, and an `$effect` that starts/stops the `socialBadge` poll. | Low. |
| `web/src/lib/components/navigation/MobileNav.svelte` | `/social` item (Users icon) after `/list`, grid `grid-cols-5` → `grid-cols-6`, unread dot. | Medium if the reading slice also adds a mobile tab: the column count must match the item count. |
| `web/src/routes/item/[id]/+page.svelte` | import `TitleSocial`; one block after the main `{#if}` chain renders `<TitleSocial target={{ item_id: item.id }} />` for every item, video or reading. | Low: appended after the page body, nothing inside existing markup changed. |

Behavior notes for the reading slice:

- Reading progress already flows through `PUT /api/items/{id}/progress`
  (D-085), so comic/manga reading shows up in the activity feed with no
  extra work. If the reader adds another progress write path, call
  `s.recordSocialProgress(userID, prev, next)` after the write.
- Social records reference works by `kind` + normalized title. New kinds
  need nothing from this slice; kind must be a lowercase token
  (`[a-z0-9-]{1,24}`).
- Title pages for comic/manga items get the social panel automatically.
  Its single-key shortcuts are `r` `s` `c` `f` `m` on `/item/{id}`; if the
  reading slice adds page-level keys there, avoid those letters (the
  reader routes `/read/*` are untouched).
