# Shared changes

Edits a slice made outside its own files, so parallel slices can avoid
or resolve conflicts. One section per slice; one row per touched spot.

## Social (`feat/social`)

Everything else the slice adds is in new files: `internal/contracts/social.go`,
`internal/kv/social.go`, `internal/plugins/social/`, `internal/gateway/social.go`
(+ tests), `docs/slices/social.md`, and new web files listed below.

| File | Change | Conflict risk |
|---|---|---|
| `internal/core/composition.go` | +4 bindings (`lain.social.*@1` → `lain-social-bolt`) appended after `lain.settings.integrations@1`, behind a comment. **No `Version` bump** — `Upgrade` adds missing capabilities on its own, so the reading slice keeps the next version number. | Low: append-only inside the bindings map. |
| `internal/gateway/server.go` | import `plugins/social`; `social.New(db)` after `list.New`; `reg.Register(soc)` after `reg.Register(lst)`; `s.routesSocial()` after `s.routesList()`; one line in `handleProgressPut` after the scrobble block: `s.recordSocialProgress(v.UserID, prev, in)`. | Low: five one-line insertions next to list-slice lines. |
| `internal/kv` | **Not edited**: social buckets are declared in the new `internal/kv/social.go` and created by the social provider, so `kv.Open`'s bucket list is untouched. | none |
| `docs/CONTRACTS.md` | New `## Social` section inserted right before `## Component mode (wire protocol)`. | Low. |
| `docs/advisor/decisions.md` | **Not edited**: social choices are `S-*` in `docs/slices/social.md` until merge, so neither slice grabs the next `D-` number. | none |

Behavior notes for the reading slice:

- Reading progress already flows through `PUT /api/items/{id}/progress`
  (D-085), so comic/manga reading shows up in the activity feed with no
  extra work. If the reader adds another progress write path, call
  `s.recordSocialProgress(userID, prev, next)` after the write.
- Social records reference works by `kind` + normalized title. New kinds
  need nothing from this slice; kind must be a lowercase token
  (`[a-z0-9-]{1,24}`).
