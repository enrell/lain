# Social slice — plan

Status: in progress on `feat/social` (worktree `../lain-social`).
Authorized directly by the user (2026-10-03). No `advisor` subagent is
available in this harness, so the slice cites existing decisions and
records its own load-bearing choices below as `S-*` (to be promoted to
`D-*` numbers when merged, so the parallel reading slice cannot collide
on the next `D-` id).

## Goal

Let accounts on one Lain server connect: profiles, friends, an activity
feed, sharing and collections, ratings/reviews/comments, notifications,
and privacy controls over all of it — for **any** media kind (anime,
series, movies, and the comic/manga kinds of the reading slice, D-085).

## Choices (load-bearing)

- `S-1` **A fourth domain.** Social is its own domain beside catalog,
  userstate and the list (D-008, D-078 extended): its own buckets, its
  own provider `lain-social-bolt`. It never writes catalog, progress or
  list records; it only *observes* progress writes through a gateway
  hook to record activity.
- `S-2` **Capabilities, not gateway islands** (D-075/D-076). Four
  exactly-one capabilities served by `lain-social-bolt`:
  `lain.social.graph@1` (privacy settings, favorites, relationships),
  `lain.social.activity@1` (activity log, feed, notifications),
  `lain.social.reviews@1` (ratings, reviews, comments),
  `lain.social.collections@1` (collections, shares). Handlers stay thin
  and route through the registry. The composition gains the four
  bindings; `Composition.Upgrade` adds them to saved compositions
  without a version bump (keeps the reading slice free to bump).
- `S-3` **Media-agnostic work identity.** Social records reference a
  *work* (`contracts.WorkRef{kind, title, item_id?}`), keyed by
  `kind + "\x00" + catalog.TitleKey(title)` — the same grouping rule the
  title page uses (D-056), split by kind so "Frieren" the manga and
  "Frieren" the anime keep separate ratings. `kind` is any lowercase
  token (`[a-z0-9-]{1,24}`), so new media kinds need no change here. A
  client may name a work by catalog `item_id` (the gateway resolves the
  kind and title) or by `kind`+`title` (list entries, D-078, which may
  have no local file).
- `S-4` **Audience is one server.** "Public" means every signed-in
  account on this server; nothing is federated or anonymous. Every
  social route requires auth.
- `S-5` **Privacy model.** Per-account settings with three levels
  (`public` / `friends` / `private`) for: profile details (bio,
  favorites, collections list), activity, and ratings/reviews; plus
  `discoverable` (appear in user search, default on), `allow_requests`
  (`everyone` / `nobody`) and `hide_progress` (feed shows completions and
  ratings but not "watching/reading now"). Defaults: profile `public`,
  activity `friends`, ratings `friends`. Display name, username and
  avatar stay visible to signed-in users (they already are, D-086).
  Blocking is symmetric for visibility: neither side sees the other's
  content, requests and shares are refused, and a blocked user is never
  told they are blocked (the blocker reads as "unknown user").
  Admins get no privacy override in the API.
- `S-6` **Activity from progress, deduplicated.** The gateway's
  progress paths (HTTP PUT and the supervised local player) report each
  write to `lain.social.activity@1`; the provider records `progress`
  at most once per work per 6 h and `completed` only on the
  not-completed → completed transition (the D-084 rule). Ratings and
  reviews record their own activity. Each account keeps its newest 500
  activity rows.
- `S-7` **Comments are explicit public acts** on a work, visible to every
  signed-in account except across a block; authors delete their own,
  admins may delete any (moderation). Bodies ≤ 1000 runes, reviews ≤
  2000, collection names ≤ 80, notes/messages ≤ 280, validated with the
  same rules as profile text (no control characters).
- `S-8` **Shares and notifications.** Sharing a work or a collection
  targets friends only and lands as a notification. Notification types:
  `friend_request`, `friend_accepted`, `share`, `collection_shared`,
  `reply`. Each account keeps its newest 200.
- `S-9` **Collections.** Named, ordered lists of works (≤ 500 items),
  visibility `private` / `friends` / `public`, plus explicit sharing with
  chosen friends (read-only for them). Independent of the lain list:
  the list tracks status, a collection curates.
- `S-10` **Favorites** are up to 12 works on the social profile record
  (not the account record — auth stays owner of identity, D-005/D-086).

## API (all `requireAuth`)

```
GET    /api/social/me/settings            PATCH /api/social/me/settings
GET    /api/social/users?q=               GET   /api/social/users/{id}
GET    /api/social/users/{id}/activity    GET   /api/social/users/{id}/ratings
GET    /api/social/users/{id}/collections
GET    /api/social/friends
POST   /api/social/friends/{id}           (request, or accept an incoming one)
POST   /api/social/friends/{id}/decline
DELETE /api/social/friends/{id}           (unfriend or cancel request)
POST   /api/social/blocks/{id}            DELETE /api/social/blocks/{id}
GET    /api/social/feed?before=&limit=
GET    /api/social/notifications          POST  /api/social/notifications/read
GET    /api/social/work?item=|kind=&title=
PUT    /api/social/work/rating            DELETE /api/social/work/rating?...
POST   /api/social/work/comments          DELETE /api/social/comments/{id}
POST   /api/social/share
GET    /api/social/collections            POST  /api/social/collections
GET/PATCH/DELETE /api/social/collections/{id}
POST   /api/social/collections/{id}/items DELETE /api/social/collections/{id}/items?...
POST   /api/social/collections/{id}/share
```

## Web

- `/social` — Feed / Friends / Notifications (keys `1` `2` `3`, `/`
  focuses user search), unread badge on the nav link.
- `/u/{id}` — profile: avatar, bio, favorites, activity, ratings,
  collections, relationship actions with keys.
- `/social/collections/{id}` — a collection.
- Title page panel: rate (`r`), review, comments, share (`s`), add to
  collection (`c`), favorite (`f`).
- Settings → YOU → Privacy (D-087 row pattern, applies on change).

## Out of scope (this slice)

Federation between servers, real-time push (the client polls), direct
messages, reactions/likes, comment editing, importing social data from
external platforms.

## Tests

Provider unit tests (graph state machine, privacy matrix, dedupe,
caps/pruning, validation), the shared provider contract harness, and
gateway HTTP tests for every route including cross-user privacy and
block behavior.
