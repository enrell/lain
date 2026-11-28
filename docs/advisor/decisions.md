# Decisions

Reset on 2026-11-13. The earlier log is in git history
(`git log -p -- docs/advisor/decisions.md`).

## D-300 — a provider may give the operator a notice

2026-11-15, asked for by the owner. A provider that works can still have
something the operator should know: the lain-parser daemon reports a newer
model on its release channel. `Provider::notice()` returns one line (default
none); a component sends it as `notice` in its health output, the host keeps
the last one, and `ProviderInfo.notice` carries it to `GET /api/plugins` and
the plugins page. Additive on the component wire: old hosts ignore the field,
old components omit it, the recorded fixture is unchanged. The server never
acts on a notice and never downloads anything; the text is the provider's own
English sentence, like `describe` (D-285). `lain-release-model` builds its
line from `update` in the daemon's `GET /health` (lain-parser 1.4.0).

## D-301 — deleting a file leaves nothing behind

2026-11-20, asked for by the owner: deleting from the title page "is buggy,
gives no useful feedback, sometimes does not work and leaves the content on
disk, and what is deleted stays as a ghost in the interface". Three causes,
none of them an error the route reported (every delete answered 200):

- An import hardlinks or copies, so the torrent copy kept the bytes while
  the grab seeded or kept its data. `DELETE /api/items/{id}` now tells the
  acquire module, which drops that copy through `discard` (library files
  are never touched, D-112) and stops its seeding. Deleting one episode of
  a pack ends the pack's seeding: freeing the space the admin asked for
  wins over the seed goal.
- A monitored title read the deleted episode as missing and fetched it
  again. The unit is recorded in `Monitored.skipped` and is not wanted
  until the list is cleared (Acquire › Wanted, `w`). `skipped` is additive
  and server-owned on a write that omits it.
- A title whose every file is missing stayed on the lists. The owner's
  answer to Q-041 stands (missing means something only while the series
  exists); the client-side rule that implemented it was lost when paging
  moved to the server (D-283). `GET /api/catalog` and `GET /api/search`
  leave such a title out. The rows stay (D-068, D-073): nothing is
  hard-deleted, and a returning file lists the title again.

With the file go its subtitle sidecars, its own stills and the folders it
leaves empty (never the library's folder, never a folder holding anything
else). The answer carries `removal` so a client says what happened: bytes
freed, downloads stopped, "not wanted again", or that another hard link
still holds the bytes. A delete while the library's folder is unreachable
is refused (409) instead of answering 200 for a file that would come back
with the disk. Not done: a list of hidden titles for admins, and purging
long-missing rows (D-068's deferred retention question).

## D-302 — metadata is its own pass, not a scan phase

2026-11-27, asked for by the owner: "you must separate enrich from scan".
A scan of 872 files walked, identified and committed in about 100 ms, then
spent minutes in its `enrich` phase asking Kitsu, AniList, Jikan and TVMaze
about titles none of them can know (personal clips), about 0.7 s
each, every scan: "nothing found" is only cached when every provider
answered, and for a day.

- The scan ends at commit. `Scanner` runs the backfill on its own thread
  (`lain-enrich`) with its own status (`ScanStatus.enrich`, `EnrichStatus`)
  and event (`enrich-state`); a scan may start while it runs, and a pass
  asked for meanwhile runs once after it. `POST /api/library/enrich`
  (manager) starts one. The `enrich` scan phase and
  `progress.enrich_total/enrich_done` are gone; `stats.enriched` stays,
  always 0, for older clients.
- `Library.local_metadata` ("Fetch metadata online" off): the library's
  items are enriched like a private library's (D-268), from `.nfo` only.
  Default off, so every existing library keeps fetching.
- An item every network provider answered "unknown" for is remembered
  (`m\0<item>` in the metadata cache table) and left out of later
  backfills until one asks with `retry`; a skipped (failed) provider
  never makes a miss, and a manual `POST /api/catalog/{id}/enrich` always
  asks. Items of local-only libraries are never remembered (a file read).

## D-303 — books: EPUB, PDF, MOBI and AZW3 in the read module

2026-11-27, asked for by the owner: "full book support ... EPUB, PDF (our
own library written from scratch in Rust), MOBI (converted to EPUB) and
AZW3 (likewise). Online search stays out". This answers DESIGN-v2 §12 Q4
and amends D-085 (which left `pdf` unread) and D-195 ("no PDF").

- The read module (D-154) reads `manga`, `comic` and now `book`: its
  capabilities are `lain.comic.pages@1` and `lain.book.manifest@1`, its
  routes `/api/items/{id}/pages*` and `/api/items/{id}/book*`, so
  disabling it unmounts both.
- No online metadata for books, ever (D-308). Audiobooks stay out (D-016).
- The web saves books for offline reading like comics (the manifest,
  chapters and their resources, or a PDF's pages at 1536 px, which the
  service worker serves at any width asked).
- The desktop and Android apps read PDF books in their page readers
  (`/pages`) and reflowable books natively: the desktop lays the
  sanitized chapter out in GPUI (its own HTML-to-blocks reader,
  `apps/desktop/src/book.rs`), Android shows it in a WebView with
  JavaScript, file and content access off (`LainCore::book_chapter`
  gives it absolute, token-carrying resource URLs and
  `lain-chapter://n#anchor` links). Both save the same virtual pages as
  the web (`book_progress_at`/`book_position_of`). A book downloaded to
  the device is read from its own file with `lain-read` (Android inlines
  its pictures as `data:` URIs for the WebView; PDFs are drawn on the
  device at 1536 px), so both apps read books with no server.

## D-304 — `lain-pdf`, an in-tree PDF library

2026-11-27, the owner's order (D-303). A workspace crate with no crate
beyond `miniz_oxide` (DEFLATE, PNG) and `sha2` (the AES-256 revision's
hash): the file structure with xref tables and streams, object streams,
incremental updates and a rebuild by scanning when the xref is broken; the
standard security handler for the empty user password (RC4, AES-128/256,
MD5 in-tree); the stream filters, a JPEG decoder and encoder, a JPEG 2000
decoder (Part 1: JP2 and raw codestreams, every progression and code-block
mode, both wavelets, palettes), PNG, CCITT G3/G4 and JBIG2; TrueType, CFF (CID-keyed too) and Type 1 font programs,
the standard encodings and the Adobe Glyph List; simple, composite and
Type 3 fonts with CMaps; colour spaces, functions, shadings (meshes
included), tiling patterns, transparency groups, soft masks, optional
content and annotation appearances; an exact-area anti-aliased
rasterizer with a stroker. A page is drawn as a picture (`render_page`,
then JPEG when pictures cover most of it, else PNG), so every client reads
PDFs the way it reads comic pages.

- Fonts a PDF names without embedding are drawn with installed
  look-alikes (`font::system`: URW base 35 first, the standard 14's metric
  clones; then Liberation, the usual Windows and macOS names, DejaVu). The
  Docker image installs `font-urw-base35`; with none installed, the
  manifest warns `fonts-missing` and the text is not drawn.
- Work is bounded: an operator budget and a deadline per page, capped
  images, pages, decoded streams and nesting; hostile input is a typed
  error, never a panic. Rendered pages are cached in memory (128 MB).

## D-305 — EPUB, and MOBI/AZW3 converted to EPUB

2026-11-27 (D-303). An EPUB is read in place (container, package
document, EPUB 3 navigation or NCX, IDPF and Adobe font obfuscation). A
MOBI (MOBI 6/7, PalmDOC and HUFF/CDIC) or AZW3 (KF8, joint files
included) is converted to an EPUB 3 in memory and served exactly like an
EPUB; the conversion is deterministic and takes tens of milliseconds.
DRM (encrypted MOBI records, or an EPUB whose content documents use an
encryption other than font obfuscation) is refused as `drm-protected`.

- A chapter is sanitized on the server: rebuilt as HTML from an
  allowlist of elements and attributes; scripts, handlers, forms, frames
  and every URL leaving the book are dropped; links between chapters carry
  `data-lain-chapter`/`data-lain-anchor`; resources point at the
  resource route; the chapter's stylesheets are inlined with their URLs
  rewritten. Chapters and resources are served with
  `Content-Security-Policy: sandbox …` and the web shows them in a frame
  that runs no script.
- Converted books are kept in memory (bounded, 512 MB with PDFs), never
  written to the data dir: nothing to back up (D-197) or clean (D-301).

## D-306 — the `book` kind, its routes and the readers

2026-11-27 (D-303). A library type `book` (`epub`, `pdf`, `mobi`, `azw3`);
`auto` (D-270) takes them too. The book identifier
(`lain-identify-book`) runs before the comic and anime ones (composition
version 11 adopts it into saved documents) and declines every other
extension, so the parity fixture does not move; the kind is always
`book`. Routes (D-169, conformance `read/00-read.json`):
`GET /api/items/{id}/book` (the manifest), `/book/chapters/{n}` (a
sanitized chapter), `/book/res/{*path}` (a resource, `?token=` like
pages); a PDF book is read through `/api/items/{id}/pages` (sizes in
points) and `/pages/{n}?w=` (rendered `w` pixels wide). The web reads a
PDF with the comic engine (ReaderBase) and a reflowable book with a
reader of its own: paged in columns or scrolled, contents, text size,
font, colours, spacing and margins (personal, applied at once), and the
reading keys of D-086. Integrations may send `book` (D-266); `lain watch`
refuses it like a comic.

## D-307 — reading progress of books in virtual pages

2026-11-27 (D-303), amending D-085's page/page-count rule for reflowable
books only: a page there depends on the screen, so progress is counted in
virtual pages of 1024 characters (`BOOK_PAGE_CHARS`): `position_sec` is
the 1-based page, fractional to keep the exact place, `duration_sec` the
book's page count, so "page 183 of 512" reads the same on every client.
Characters are counted by a versioned rule (`text@1`: non-whitespace
characters of the sanitized chapter's body), named in the manifest's
`count_rule`. A PDF keeps D-085's real pages.

## D-308 — book metadata comes from the book only

2026-11-27 (D-303). The enrich pass (D-302) asks `lain-metadata-book`, a
local provider like `.nfo` files, for a book's own title, description
(`synopsis`), subjects (`genres`) and year; `book` items are local-only
whatever their library says, and never remembered as misses. The overlay
stays as D-032 froze it: authors, language and publisher are in the
`/book` manifest. Covers come from the book (its cover image, or a PDF's
first page) through the thumbnail route.

## D-309 — `auto` libraries take books

2026-11-27 (D-303, D-270). `auto` is "a folder that holds more than one
kind", so it takes the four book formats with the video and comic ones.
An existing `auto` library imports the books (and PDFs) its folder holds
on its next scan.
