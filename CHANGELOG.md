# Changelog

All notable changes to the Lain media server are documented here.
Format follows [Keep a Changelog](https://keepachangelog.com/en/1.0.0/).

## [Unreleased]

### Added
- **Indexer presets** (D-290): Settings › Indexers › Add indexer asks a
  Prowlarr or Jackett you run for the indexers it serves and adds the
  ticked ones in one step, with its API key; a feed URL typed by hand can
  be tried first (caps and a few latest releases). `POST
  /api/acquire/indexers/discover`, `/import` and `/probe`. There are no
  public presets: no well-known public source publishes a stable feed of
  its own.
- **Set up indexers with a coding agent** (D-290, D-291): one button (key
  `a`) makes an integration key that only manages indexers, expires (15
  minutes to a day) and can be revoked from the same dialog, and shows a
  prompt for Claude Code, opencode or a DeepSeek harness that points at
  the server's own guide, `GET /api/agent/indexers.md`, and tells the
  agent to ask before adding anything. `POST /api/integrations` takes
  `scopes: ["indexers"]` and `ttl_seconds`; such a key is accepted only
  in the `Authorization` header and only by `/api/acquire/indexers*`.
  The empty Indexers page and Acquire's first step offer both ways.
- **Pages of titles** (D-283): `GET /api/catalog` and `GET /api/search`
  take `group=title`, which pages titles instead of files: `total`,
  `limit` and `offset` count titles and `items` holds every file of each
  title on the page, so a title is never split and a search says how many
  titles match. `sort=recent&group=title` lists the newest titles.
  `lain-client`: `CatalogQuery.group`, `SearchQuery.group`.
- **An episode's own picture** (D-284): `GET /api/items/{id}/thumbnail`
  takes `art=still`, the file's own picture (`<name>-thumb.*`, else its
  frame), never the show's poster kept beside its first episode. The web
  asks for it in every episode row and Continue watching card, so Episode
  1 no longer shows the show's poster cut to 16:9. `lain-client`:
  `Client::still`.
- **Delete a user** (Settings › Users, `DELETE /api/users/{id}`): the
  account goes with its progress, tracking lists and linked accounts,
  ratings, comments, collections, friends and notifications; private
  libraries it owned pass to the admin deleting it. The dialog names
  what goes and asks for the username typed out. A later account of the
  same name never accepts the deleted one's tokens.
- **Edit and order libraries** (Settings › Libraries): rename, change
  the type or the folder, or make one private from the dialog that adds
  libraries; move them up and down (Alt+arrows on a row). Home and the
  library page follow the order. `PATCH /api/libraries/{id}` takes
  `type` and `path`; `PUT /api/libraries/order` sets the order.
- **Plugins name every job** (D-285): each binding on `GET /api/plugins`
  carries a one-sentence `summary` from the server (a component can
  describe the jobs it brings in its manifest). Settings › Plugins shows
  it for every row, in families that close to one line each (how many
  jobs, and whether a provider is missing or unhealthy), and says how
  to install a provider that is bound but missing.
- **`lain restore BACKUP.tar --replace`**: with the server stopped, one
  command moves the data folder aside (kept as `<dir>.before-restore`)
  and restores into a fresh one, keeping this machine's installed
  components and service wrapper; a failed restore puts it back.
- **The player's episode list on every screen**: between an upright
  phone (list under the picture) and a wide screen (list beside it) the
  bar's list button and E open it as a drawer over the picture's edge,
  which keeps Tab inside it; Escape, the button or a click outside close
  it. Beside the picture the list has its own Hide button with its key,
  the choice is kept, and the episode playing is scrolled into view.
- **Reading options on a phone**: the reader's top bar is one row, and
  its reading toggles open under it from one button.
- **Rails page with a mouse** (web): Home's rows, the library's shelves
  and "More in" show a previous and a next button at the window's edges
  on hover while that side hides more; the arrows still walk them card by
  card.
- **Back returns the focus to the card that was opened** (web): Search, a
  library grid, the shelves and Home put the focus back on the title you
  opened, not on the page; the focus ring shows when you came by keyboard.
- **The library's view is in the address** (web): `/library?view=all`
  opens All titles and can be linked; Back returns to the view you left,
  and the browser's last choice is only the default for a bare `/library`.

### Changed
- **Web: titles, not files.** Home lists each library's newest titles in
  its own row (a show is one card counting all of its episodes, not a run
  of "Glass Garden · 03" cards), and the mixed Recently added row is gone;
  the Favorites rail lists whole titles. Search counts the titles that
  match, not the ones loaded, and never cuts a show at the end of a page.
- **Web: one frame per picture shape.** A grid whose titles mix posters
  and wide stills (All titles, a search, the favourites) is in two parts,
  posters and covers then widescreen, each at Home's sizes, instead of
  letterboxing one shape inside the other's frame; Search's newest titles
  split into two rails the same way. Continue watching shows a film's own
  wide still instead of its poster between blurred bars.
- **A scan asked for during a scan is queued on the server** (D-286):
  `POST /api/library/scan` answers 202 with `queued: true` instead of
  409, and the follow-up starts when the running scan ends, even after
  the tab that asked is closed. Settings › Libraries shows it and lets
  you queue one from Scan now.
- **Settings › Transcoding fits on one screen**: what the probe found
  (the video engine, HDR) and the Auto / Custom policy at the top, every
  custom section closed to a line of its current values. A server with
  no hardware encoder says it encodes on the CPU instead of offering a
  one-option control, and a fresh server reads as Auto.
- **One segmented control** across Settings and the library's By library
  / All titles switch; Settings group headings use the app's section
  heading size, and the gap under every page title is one value.
- **A wrong current password has a code**: `PATCH /api/me/password`
  answers `code: "wrong-password"`, which Settings › Security reads
  instead of the message.
- Settings › Profile always shows whether the name and bio are saved;
  your own row in Settings › Users shows your role as text instead of
  disabled controls; library tags are one style and a phone's library
  rows no longer wrap Remove; every Settings string goes through `t()`.
- Settings rows stack beside the section rail on a tablet (768 to 959
  px), and the Libraries and Users tables start at 1280 px, so a name or
  a hint is never squeezed to a few letters.
- **The reader fits a whole page between its bars** while they show,
  and gives it the full height when they go away; full width, zoom and
  the strip still scroll under them. The spread's Cover alone is drawn
  as a check box option of the spread, in plain words after a hairline,
  not a second filled pill.
- **The player's settings panel fits a phone on its side**, and its
  menus share one width. The reader's `?` list is the app's dialog, as
  the player's is.
- Every string of the player and the reader, their settings panels and
  their routes, and a readable file's name (Vol, Ch, Issue), goes
  through `t()`.
- **The error page** (web) opens like the first-run pages, with the status,
  the address set apart, and keys on every way on (Enter Home, L Library,
  Backspace Back when you came from inside Lain, R to retry a failed load).
- **First-run pages each have their own art** (web): Tracking draws a list
  kept elsewhere, Social a title among three people, Acquire releases on
  their way in, all from your own titles, beside the words from 768 px.
- **One meaning per key** (web): `/` searches (the header's titles, or the
  search of the page you are on); Social finds people with **F**, and
  Acquire's "Grab from a link" is **L** from any tab, no longer `2`, the
  Queue tab's key.
- **Favorites with few titles** (web) keep their filters in the toolbar's
  sheet; the filter column comes from 12 titles up.
- **Login and Setup share one field style** (web), and sign-in says how
  to get back in after a forgotten password; a disabled account is told
  so, not that its password is wrong.
- **Web player: one play button.** With a mouse the bar's play button is
  the only one (a click on the paused picture still plays); on a touch
  screen play and the two skips sit in the middle of the picture and the
  bar keeps the time and the other buttons, all one size. An upright
  phone stacks the player: the picture under the top bar, the controls
  under the picture, so "Resuming from…" no longer covers the frame;
  there the settings panel drops from the gear over the episode list (a
  tap on the list closes it) instead of covering the top bar and title.
- **Web on a phone:** a title page's Back sits in the top bar, where it
  stays on screen (named "Back to the library" when the title was opened
  from outside Lain, where that is where it goes); the reader shows its direction (← RTL, a tap flips it)
  beside the page slider; Settings › Libraries and Users keep each row's
  actions in one labelled ⋯ menu (Edit, Move up, Move down, Remove;
  Playback, Reset password, Delete) instead of bare icons; the library's
  Unlock, Scan and Manage libraries are one ⋯ beside its heading, and the
  settings section picker ends with Sign out.
- **Web: one card size per shape.** Posters and widescreen cards are one
  size in every rail and grid (Home, the library's shelves, More in, a
  library, the favourites, search results): a grid fills its width with
  the column count nearest the rails' size, so a library grid beside its
  filters no longer draws smaller posters than the rail under a title.
- **Web: All titles says where its count continues** ("33 titles · 12 in
  widescreen"), a jump to the part further down; the first part's own
  heading sits just under the line.
  The browse toolbar's sheet button says "Filters" where the sort and tile
  size sit beside it (the sheet then holds the filters alone), and the
  sort carries an icon like the toolbar's other controls.
- **Web: Search before a query** puts Recent and Browse side by side, ends
  the recent searches with their Clear, and says what a search matches
  under the heading, so the newest titles' rails start in the first screen;
  once there is an answer, its count takes that line.
- **Web: Settings › Transcoding's HDR row is a switch** whose hint names
  the tone map that runs, instead of a bare "HABLE"; turning it off
  switches the policy to Custom. The server's rows are the one place the
  video engine and HDR on or off are set, in Auto and Custom alike; the
  Hardware and HDR sections keep the rest of each, and the page's filter
  finds the server's rows too; with an ffmpeg that cannot tone map, the
  HDR section says so instead of offering an algorithm and a peak that
  never run. **Settings › Plugins** says a family's
  optional components in words after its health ("5 jobs · all healthy ·
  3 optional, not installed"), each part on one line at 360 px.
- **Web:** a title's File details closes the hero under its synopsis; the
  player's episode list fades where more episodes are hidden; a volume
  under way says its page on Continue reading ("Vol 1 · page 4 of 8").

### Fixed
- **Web:** films and clips without a year are captioned with their length,
  not "video"; time left is said one way on every card and hero ("9 sec
  left", where a card said "1 min left"); a film card under way says what
  is left.
- **Web, series page:** Resume picks the episode watched last when several
  are half-watched, as Home does (one Continue watching card a title); on
  arrival the page scrolls just far enough to show that episode, which
  wears the accent; the phone hero is tighter and its synopsis clamps.
- **Web, film and comic pages:** a watched film says so ("Watched 2 days
  ago") and offers Watch again; a comic shows its ComicInfo.xml summary and
  genres in the hero; volume cards are the size of the rail cards under
  them.
- **Web, title pages:** a card in a title's "More in" rail opens that
  title; the page kept the first title on screen under the new address.
- **Subtitles step above the player's controls in Firefox**: Firefox
  draws cues where they cannot be moved, so there the player draws them
  itself (the D-082 overlay), above the controls while they show. The
  overlay's cues are sized from the player's height, as the browsers'
  own are.
- **The player's on-screen messages reach screen readers**: one live
  region stays mounted and only its text changes.
- **The reader's newest page is saved when the tab closes**, by rules
  now covered by tests; going back to the page a volume opened on is
  saved too.
- The player and the reader no longer show the page's scrollbar gutter
  as a grey strip at the right edge, and the Enter key on the reader's
  end card is readable.
- **Library on a phone** (web): the shelf names get their own row under
  the view switch (the fourth was cut), and a row of names fades only
  where names are hidden, at its start too once scrolled.
- **Social** (web): Privacy settings appears once; a tab shows it is
  loading until its first answer instead of flashing its empty state;
  Friends with nobody yet is drawn as an empty state.
- **Acquire** (web): key hints spelled out in words ("Enter grabs") are
  hidden on a touch screen; empty search results, Blocklist and Subtitles
  sections are drawn as empty states.
- **The phone's search field** (web) is filled like the library's and the
  favourites' fields.
- **Web:** every card's progress bar sits on a dark fade, so it reads on
  a pale cover; a title's hero says what each count measures ("Episode 1:
  10 sec left · 6 of 12 episodes watched"), and a synopsis cut short ends
  in More; the profile card shows the display name as typed, not in
  capitals; a title's action row and Acquire's five tabs fit a 360 px
  phone; the comment box's key hint no longer leaves "to write" alone on
  a touch screen; the player's Retry and Back on a failure or a transcode
  wait take clicks and taps (a touch layer above them took them), and
  their words sit between the top bar and the controls on a phone.
- **Web:** File details' "Metadata from" line lost the space before its
  date ("lain-import· Nov 11").
- **Web:** an episode watched for a fraction of a second wears the card's
  progress sliver beside its time left (its percent rounded to 0); the
  profile card's facts no longer end a phone's line on a lone "·";
  Settings › Users words Delete like Playback and Reset password from
  1280 px; the library's bar sits closer under the heading on a phone;
  Acquire's and Tracking's first steps say what an indexer or a link is
  rather than repeating where the button above goes, and their step
  numbers set the dot evenly; Indexers' empty list says Acquire has
  nothing to search; Acquisition's engine status fits one line; a
  favourite, rating or activity made from a show reads Series (and a
  film or clip Video) instead of the raw "EPISODE" or "VIDEO".

## [0.3.1] - 2026-11-12

### Added
- **Catalog answers say what each thumbnail is** (D-282): `GET
  /api/catalog`, `/api/catalog/{id}`, `/api/catalog/{id}/episodes` and
  `/api/search` give every item an additive `thumbnail`: `sidecar` (a
  still that came with the file, D-269, with its `aspect` read from the
  picture's header), `cover` (a comic archive's cover page) or `frame`
  (cut from the video). Filled on each read, never stored, so a poster
  added next to a file counts at once.
- **`https` servers in every native client** (D-277): `lain-client` speaks
  TLS through `lain-http`'s rustls setup, so `lain login --server
  https://...`, the desktop app and Android reach a server behind a reverse
  proxy. A failed handshake is `ClientError::Tls` (its own message on
  Android).
- **Signed Android releases** (D-278): a `v*` tag attaches
  `lain_<version>_android.apk` and its `.sha256` to the release, signed
  with the project's key; the version code comes from the tag
  (`v0.3.1` is 3001), so every release installs over the one before.
- **Release alternatives** (D-279): a release name that reads more than
  one way ("Show 2 - 03": episode 3 of the sequel, or of season 2) is
  settled by the title the server expects. With lain-parser 1.3.0 the
  `lain-release-model` provider asks for up to five other readings; a
  search takes the one that names its query, automation and its grabs
  the one that names the monitored title or an alias, and an import the
  one that names the grabbed release; a reading whose year is more than
  one from the expected year is never taken. A release the parser
  answers as a monitored title stays that title's: with "Show" and
  "Show 2" both monitored, "Show 2 - 03" never becomes season 2 of
  "Show". A manual grab keeps the parser's answer, and a library's other
  titles never pull a sequel into the show it follows.
  `GET /api/acquire/parse` shows the readings (`alternatives`,
  additive); they are never stored in a grab or returned in a search.
  An older parser service, or the tokenizer, offers none and nothing
  changes.
- **Audio language and subtitle language apart** (D-280): Japanese audio
  with Portuguese subtitles, and no subtitles over a film dubbed in
  Portuguese. The account keeps an audio language, a subtitle language
  and when subtitles show (Auto: when the audio is not in your subtitle
  language; Always; Forced only; Off). Settings > Playback on the web and
  Settings in the Android app have the three rows; the web player, the
  Android player, mpv and VLC opened by the server and `lain watch` pick
  tracks with them, and a choice made in the player for a title still
  wins (the web player now remembers it per title too). A transcode
  started without a track choice plays the preferred audio language, so
  the Android app hears it in an HLS session. `lain language` gains
  `--audio`, `--subtitles` and `--subtitle-mode`.
- **API**: a player can name itself with a `viewer` id when it starts or
  joins a transcode session and when it releases it (D-281). Two players
  with the same options share one session; it now stops only when the
  last of them leaves, and a finished one stays cached. Requests without
  a viewer behave as before. The web player sends one; `lain-client` has
  `release_transcode`.
- **First-run pages for Tracking, Social and Acquire** (web): on a server
  with nothing of yours there yet, each says in a line or two what it is
  for and offers the first step as its main action, with a key (connect
  AniList, or sync a linked account that came back empty; find people and
  the privacy settings; add an indexer, or a library first), over art from
  the server's own library, in place of a grey box.
- **`?` lists the keys of the screen in view** (web), above the keys that
  work everywhere: a title's P (play or resume, new), F, R, S, C, M, D and
  T, the library's filters and grid keys, Social's and Acquire's tabs, the
  settings chords and row keys, each only while it does something. A page
  filter that takes `/` hides the header's `/` cap.
- **More keys** (web): Left and Right walk a row of cards (Home, the
  library's shelves, "More in"); the arrows move between a title's
  episodes and volumes; Ctrl+Z takes the newest toast's Undo; in the
  player, C turns subtitles on and off, < and > set the speed and E hides
  the episode list beside the picture (also a bar button, remembered).
- **A page for unknown addresses** (web), with Home focused and the
  library beside it, in place of a bare "404 Not Found"; `/tracking`
  opens Tracking.

### Changed
- **A server address typed without `http://` or `https://`** (D-277) is
  `http` on port 9360 when it is local (a private, loopback, link-local
  or tailnet address, a one-word name, or a `.local`, `.lan`,
  `.home.arpa`, `.internal` or `.localhost` name) and `https` on port 443
  otherwise; an `https` that fails is never retried as `http`. This holds
  in the CLI, the desktop app and Android, whose connect field no longer
  starts at `http://`.
- `preferred_language` stays for older clients: setting it sets both
  languages, and it now reads as the subtitle language when one is set.
  Accounts saved before keep exactly their old behaviour.
- **Search results look like a library** (web): one card frame per answer,
  the library's sizes (those of Home's rails) and the same treatment of
  stills and posters, where results were rows of mixed shapes. Arrow keys
  move between results and from the header's field into them; up from the
  first row goes back to the field. The library grid answers the same keys.
  Before a query the page offers the libraries as chips, as it does after a
  short answer, and a rail of the newest titles.
- **Social's feed opens with its titles** (web): what friends watch and
  read is a rail of cards at Home's sizes over the activity list. The
  people search shows its key in the field; spelled out in the
  placeholder it ran off a phone's width.
- **Home shows posters whole** (web): a library's row takes the frame its
  pictures decide (a films library of posters is a poster rail, not
  widescreen cards cropping each poster), and an upright picture in a
  widescreen card is shown whole over a blurred copy of itself.
- **Tracking's grid uses the library's card sizes** (web), and a wide
  cover is shown whole instead of cropped to a poster.
- **The library's view switch stays put** (web): By library and All titles
  hold it at the same place, at the end of the bar's first row; in All
  titles it sat a row lower, under the search. With the switch beside the
  search, sort and tile size join the filters sheet below a wide screen.
- **One heading scale** (web): a page's own name (Library, a library,
  Favorites, Search, Tracking, Social, Acquire, Settings, a collection, a
  profile) is set at Settings' size, and a section under it (a rail, a
  title's episodes or volumes, More in, a search's suggestions) at Home's
  rail size. Library named its page at 20 px over 24 px rails, and a
  title page's sections were 18 px. Acquire and Social keep their one-line
  lead on every tab: hidden on the first-run tab only, it made the tabs
  under it jump each time that tab was left.
- **Episode and volume cards look like every other card** (web): the
  picture in a rounded frame with a faint ring, the words under it, and
  the inset progress bar the library's cards wear; a bordered box around
  picture and words made them a second kind of card. Volumes are sized as
  every poster grid is: two a row on a phone, where three were cramped.
- **One main action on a title page** (web): Rate, Share and Add to
  collection under Community, and a comic's Save offline, are the quiet
  buttons of the hero's row; an accent Rate competed with Play.
- **The library's toolbar is one kind of control** (web): the search,
  sort, tile size and Sort & filter are filled fields like the sort
  picker, where an outlined field, a filled picker and an outlined button
  stood side by side. The genre finder and the library picker in the
  filters end where the rows under them do.
- **The library's header on a phone** (web): the admin actions keep their
  words on one line ("Manage libraries" broke inside its chip), and the
  row of library names fades at its end, so a name scrolled past the view
  switch reads as more to swipe to rather than missing.
- **Empty tabs** (web): Acquire's Queue and Wanted and Social's
  Notifications and Collections, when empty, show the app's empty state
  (an icon, what the tab holds, how to fill it) instead of one grey line.
- **Settings controls** (web): Transcoding's video engine is one segmented
  choice like the policy above it (the engine in use, the ones the probe
  passed, software), where a mono word sat beside two bare "Use …" buttons;
  Playback's Render on is as wide as every other picker in its column; a
  plugin job's description wraps to two lines instead of stopping mid-word.
- The reader's toggles keep a gap between them (web): Spread and Cover
  alone, both on, read as one block.
- **A title page on a phone starts with its episodes in view** (web): the
  cover sits beside the title's name instead of above it, so the first
  episodes or volumes show under the actions where the hero filled the
  whole screen. The name is sized to the column beside the cover, so a
  long word ("Monochrome") fits it instead of running into the edge.
- **First-run pages on a tablet** (web): Tracking, Social and Acquire set
  their poster fan beside the copy from 1024 px; at 768 px it left the
  headline one or two words a line.
- **Play plays** (web): the player starts once the file is ready instead of
  waiting paused for a second press, so Autoplay next really plays the next
  episode. A title's Play goes on from the episode after the last one
  watched (Play · Episode 4), and Home's hero offers that episode as Up
  next, or a new title from its first episode rather than whichever file a
  scan added last.
- **Audio and quality in direct play** (web): the player's settings offer
  the file's other audio tracks and a lower quality when the browser plays
  the file itself; picking one switches to a prepared stream at the same
  place. Tracks are named by language ("Portuguese", not "por"), and the
  quality ladder stops at the file's own size, width and height alike: a
  2160p rung is not offered for a 1080p file, while a 1920x800 film keeps
  its 1080p.
- **A scan asked for during a scan runs after it** (web): Add and scan on a
  second library while the first one scanned did nothing (the server
  answers 409); the page now queues the scan, says so, and starts it when
  the running one ends. The slow metadata step says the titles are already
  browsable.
- **First run** (web): setup and sign-in focus the name field, a wrong
  password is selected for retyping, the welcome step's Add a media library
  (Enter) opens the Add dialog, and both screens go through `t()`. Dialogs
  open on their first field instead of the close button.

### Fixed
- **Web**: a title page or player on a server with no local player (no mpv or
  VLC on its machine) threw on every render: the server answers
  `players: null` there, and the Open with menu read its length.
- **A first visit draws in its final frame** (web, D-282): a films or
  shows library, All titles, search results and Home's library rows no
  longer draw wide cards that turn upright a moment later (or after
  seconds on a cold server); the frame comes from what the server says
  of each picture, and only pictures it knows nothing about are measured.
  Home's library rows are drawn with the page instead of after their
  pictures.
- **Back to a search page returns to the same spot** (web), with as many
  results loaded as before, as a library grid does.
- **An episode's still is the episode's** (web): where a show's poster
  sits next to its first episode, the episode lists of the title page and
  the player, and Continue watching, show that episode's frame instead of
  the poster cut to 16:9.
- **Home's hero shows an upright poster whole** (web): a film whose only
  art is its poster had it blown up across the hero, its lettering cut
  away; it is now shown beside the title over a blurred copy, as a book's
  cover is.
- **"New in your library" lists whole shows** (web): a show among the
  newest files counted only those files ("5 of 21 watched") and showed a
  later episode's still instead of its poster.
- **Years in acquisition** (D-279): a search with a year rejects a
  release whose year is more than one away ("year does not match"); it
  never compared years before. A model reading whose title holds the
  year ("Blade.Runner.2049.1080p" read as "Blade Runner 2049") no longer
  takes it as the release year, so a monitored "Blade Runner 2049"
  (2017) no longer refuses that release.
- Android: the audio and subtitles picked in the player for a title now
  carry to its next episode; the pick was kept per episode. A forced
  (signs) subtitle track is listed apart from the full one in its
  language.
- Android: on a video the server converts (HLS), the audio menu lists the
  file's audio tracks and a pick plays from where you are in the chosen
  language; an episode opens in the audio language last picked for its
  title instead of the account's. A converted stream carries one audio
  track, so the pick could not switch inside it.
- **Web**: a link clicked within a moment of typing in the header's
  search field on the Search page no longer bounces back to Search with
  the field focused.
- **Web**: an account without the admin role that opens an admin
  settings page lands on its own Profile settings; when an admin had last
  used Settings in the same browser it used to bounce between two pages
  forever.
- **Web**: leaving the player stops its transcode session if it is still
  being prepared and no other player (another tab or device) is watching
  it, so the next title starts at once instead of waiting behind it for
  up to two minutes; a finished session stays cached.
- **Web**: a session the player started for a seek, track or quality
  change that a second change replaced before the server answered is
  released too, instead of running on until the idle timeout.
- **Web**: pausing (K, Space) before a just-started play has begun no
  longer shows "The browser blocked playback" and pins the controls.
- **Server**: an HLS session whose hardware encode fails and is retried
  in software starts that retry from an empty directory: the failed
  attempt's playlist is withdrawn and its segments are no longer served
  next to the retry's init segment. The retry's init segment and
  segments have names of their own (`sw-init.mp4`, `sw-seg…`), so a
  player that was watching the failed attempt never decodes the retry's
  segments with the failed attempt's init segment it kept or cached.
- **Server**: a transcode session started again just as it was stopped
  (a viewer joining as the last one leaves, or a play after an idle
  stop) waits for the stopped one to finish cleaning up, instead of
  losing its files to that cleanup.
- **Grids no longer change shape while they load, nor wait for their
  pictures** (web): a library grid, a shelf, the rail under a title and
  search results are drawn as soon as their titles are here, in the frame
  guessed from what is known before a picture loads (posters in the
  overlays, reading files, and what this browser has seen of each public
  library's stills), then checked once against their first pictures and
  changed only if the guess was wrong, and held after that.
  A grid opened in one shape and turned to the other as its posters came
  in; measuring first instead left a query or a library on a cold
  thumbnail cache with nothing on screen for up to 2.5 s. A card's
  picture pulses until it loads. A provider's cover (a wide banner) is no
  longer taken for a poster.
- **Acquire no longer flashes "no library"** (web) while its libraries load.
- **Acquire's forms on a phone** (web): the link field of the Queue had
  shrunk beside the library picker to a few letters; the form stacks, and
  the labels of both forms line up whether or not they carry a key.
- **Activity rows on a phone** (web): the person and the time take one
  line and what they did the next, where the name and verb columns left
  the title a few letters; an episode of a show without seasons reads E5,
  not S0E5.
- **`/` finds people on Social again** (web): the header's search had taken
  the key since it became a field; the page now moves focus on to its
  people search once the Friends tab has drawn it, as other pages do.
- **Empty states sit open on the page** (web), an icon and a line or two
  with no grey panel around them (no search match, an empty library, a
  missing title or profile).
- **The Kind filter names kinds** (web): Films and clips, Episodes, Manga,
  Comics, where it listed the catalog's raw words.
- **Playback options and Settings > Transcoding no longer wait for the
  ffmpeg probe** (D-031): on a machine with a GPU the capability probe
  could hold a request for 20 s or more when the machine was busy, and
  every request that found no cached result ran a probe of its own. The
  server now probes when it starts, requests share one probe, and a
  result older than 10 minutes (30 s when one of its checks timed out
  or no ffmpeg was found) is answered at once while one probe refreshes
  it in the background. The probe's checks (tone mapping, video effects,
  each hardware backend) run at once. Saving the transcoding settings
  probes again in the background.
- **Staged server settings are not lost by accident** (web): Escape
  discards them only when pressed twice, and leaving the section with
  changes staged asks to apply, discard or stay; Profile's unsaved name
  and bio ask the same.
- **Confirmations before what cannot be undone** (web): revoking an API
  key and deleting a comment ask first; disabling a user and taking a
  favorite off the profile offer Undo.
- **Dialogs open on what they are for** (web): the first field, or the
  primary action, never the close button (a confirmation lands on its
  action with Enter beside it; one that deletes files lands on Cancel);
  the player's key list closes on Escape and no longer lets keys through
  to the video; the reader's takes the focus and keeps it.
- **Marking a title's episodes keeps the page where it was** (web): the
  refresh no longer swaps the page for its skeleton, which jumped to the
  top and dropped the focus; a provider swap on Plugins likewise.
- **Backup's restore steps work as written** (web): they named a `.db`
  file and restored into the live, non-empty data folder; they now move
  it aside first and name the archive just downloaded.
- **Accessibility** (web): a switch beside its label is named by it, and
  row switches by their row; Social's and Acquire's tabs follow the tabs
  pattern (arrows, one Tab stop) and are 44 px tall on a phone; switches
  take a 56 x 44 px press; a field's hint or error is read with it; the
  password and first-run forms point at the field to fix; sign-in and
  setup start in the username; `/library?view=all` opens All titles.
- **Enter in the Add library dialog adds the library** (web); it went up a
  folder and renamed the library after the parent. The dialog no longer
  names a library after the folder the picker opens on.
- **Rating with the keyboard** (web): a digit then Enter saves that score;
  Enter used to press the focused 1 instead.
- **The reader keeps the page reached** (web) when the tab is closed or
  reloaded; the last turns were lost.
- **Subtitles stay above the player's controls** (web) while they show.
- **Counts that disagreed** (web): Search's New in your library said
  "1 of 21 watched" for a 26-episode show, and a comic in volumes was
  "3 issues, Seasons 3" on its page under a card saying "3 volumes".
- **Library rows at tablet width** (web) keep their names; beside the
  settings rail the controls took the whole row.
- **An account without Acquire** opening `/acquire` lands on Home, not on
  its profile settings.
- **A prepared stream starts at the viewer's place** (web): one that ran
  ahead of playback, such as an audio switch that copies the video,
  started near the end of what it had written, up to an hour on. Leaving
  direct play no longer shows, and saves, twice the position.
- **Focus on the first screens** (web): the setup fields show which one
  has focus, and a dialog that opens on choices (the filters sheet, the
  rating) no longer focuses a field further down first.

## [0.3.0] - 2026-11-12

### Added
- **The Android app, redesigned** (#39) in the web client's language: a
  bottom bar of five places (Home, Library, Search, Downloads, Settings);
  a Home hero that names what it resumes, then Continue watching, Continue
  reading, Favorites, Recently added and a rail per library; library pages
  with search and a filters sheet; title pages whose main button names the
  episode, with the favourite heart, mark watched and downloads; a player
  that opens on its side, with gestures, subtitles and audio by name,
  picture in picture, kept speed, skip length, subtitle size and box, a
  quality choice and the next episode; audio and subtitle languages
  remembered per title; a full-screen reader (tap or swipe, pinch, three
  reading directions, an end-of-volume page); a PIN sheet for private
  libraries; downloads grouped by title. When the device's decoder fails,
  the player asks the server to transcode instead of stopping (#36).
- **The web client on a phone** (#38): a slim top bar, a bottom bar with
  More, dialogs as bottom sheets, a library filters sheet, episode rows
  and one section picker in Settings.
- **The web client, refined** (#39): one title hero for series, comics
  and films, with Manage, Open with and File details menus; favourites
  (a heart beside the main action, a Favorites page with search, sort and
  filters, a Home rail); anyone can mark a title watched or read; episode
  lengths and time left; seek-bar previews; a search field in the header
  with suggestions and recent searches (`/`), Quick find on Ctrl+K and the
  shortcuts on `?`; the library's By library / All titles switch in its
  sticky bar (`V`) with one card frame per grid; Settings in grouped
  panels, with libraries, users and plugins as tables; undo in toasts.
- **Server**: `GET /api/durations?ids=` (D-276) and
  `GET /api/items/{id}/storyboard` (D-275); scan progress on
  `GET /api/library/scan` (D-272); HEVC stream copy for browsers that
  decode HEVC (D-273); Anime4K rendered by the server into a transcode
  session (D-274); the NVENC probe uses a frame NVENC accepts.
- **Integrations and private libraries** (#37): API keys for
  integrations; a private library is enriched from local files only
  (D-268); covers that come with the files (D-269); the `auto` library
  type (D-270); composition version 9 (D-271); the optional
  `lain-library-anything` bridge (D-265..D-267); the add-library dialog
  has one path bar and a Private switch applied before the first scan.
- **The Android app v1** (P5, D-245..D-253, `apps/android`): Kotlin +
  Compose + Media3 on `lain-client` through UniFFI (`lain-ffi`). Connect and
  log in, navigation from `/api/modules`, Home rails, Library, Read, title
  pages, Search, the player (direct play, HLS transcodes, side-loaded
  subtitles, ASS burn-in) and the comic/manga reader, both resuming and
  syncing progress; offline copies within the device budget that play and
  read without the server, with offline progress synced on reconnect; the
  shipped themes. CI builds the debug APK (`lain-android-debug`).

### Removed
- **The Go server** (D-235..D-243, DESIGN-v2 §13 step 5). `cmd/`,
  `internal/`, `go.mod` and the Go CI, image and release builds are gone;
  the conformance suite runs against the Rust server only. Dropped on
  purpose: `lain bench`, `lain plugin-run`, `doctor`'s matrix report,
  `serve --transcode-cache-size` (a transcode setting now) and
  `serve --anilist-client-id` (the env seed stays). Go data, config and
  backups are not read (D-177).

### Changed
- **One `lain` binary** (D-237) replaces `lain-rs`: the server commands
  (`serve`, `service`, `export`, `import`, `backup`, `restore`, `plugins`,
  `doctor`, `component`) and the command-line client (`login`, `watch`, `download`,
  `player`, `language`, `open-url`, `install-player-handler`) on
  `lain-client`, whose progress sync now queues positions measured while
  the server is unreachable. Backups are the `lain.backup@1` tar (D-238).
- **The web UI is embedded again** (D-235): `just web` copies the build
  into `crates/lain/webui/` and the binary serves it with Go's SPA
  fallback, caching and `/api` rules.
- **Releases** (D-241) ship `lain` for Linux (static, amd64/arm64), macOS
  (amd64/arm64) and Windows (amd64) with the web UI embedded; the image
  builds the Rust binary and no longer needs `ca-certificates`.

### Added
- **Client core `lain-client`, milestone P2a** (D-179). A Rust crate in
  the workspace for every native client: the typed API for the
  conformance and desktop-parity routes (a route lain-rs does not serve
  yet is a typed `NotPorted` error), login with sessions in an
  owner-only file, manual server discovery, offline copies with D-094's
  limits and LRU eviction, progress sync that queues offline and
  resolves conflicts by explicit rules, and a self-reconnecting
  `/api/events` stream. The JSON types now live in `lain-api`, shared
  with the server.
- **Rust server, milestone P1a** (D-171). A Cargo workspace in `crates/`
  (`lain-core`, `lain-store`, `lain-auth`, `lain-component`,
  `lain-server`) builds `lain-rs`, which serves the core routes the
  clients use plus `GET /api/modules` (D-154) and `GET /api/events`
  (SSE, D-169), loads `modules.json` (`lain.modules@1`), runs Go
  components under its host, installs itself as a service
  (`lain-rs service`, D-164), and adds private libraries (D-156) and
  portable roots (D-157). It starts from an empty data dir: no Go data
  is imported, and passwords and PINs are Argon2id (D-177). It uses the
  canonical data dir (`lain`) and port 9360 (D-178). The Go code is
  unchanged.
- **Conformance suite** (`conformance/`): language-neutral HTTP cases
  and component NDJSON fixtures that both servers run.
- **Internationalization groundwork** (D-088, `docs/I18N.md`). The web UI
  routes text through a typed `t()` with plurals and `Intl` formatting,
  picks the language from Settings → Profile or the browser, and keeps
  `<html lang dir>` in sync. Pseudo-locales (`?locale=en-XA`,
  `?locale=ar-XB`) reveal untranslated text and right-to-left issues;
  `npm run i18n:report` lists what is left to migrate. English only.
- **Settings redesign** (D-087). A YOU / SERVER rail replaces the tab
  strip; personal Playback (player, language, video effects), Connections
  and Security are their own sections, and the admin transcode policy is
  now **Transcoding**, opening in Auto with Custom for every control.
  Personal settings save instantly; server settings stage behind one bar
  (Ctrl+S / Esc). **Ctrl+K** opens a launcher for every setting, section,
  action (scan, add library or user, backup, sign out) and library title;
  `g`+letter, `[` `]` and `j` `k` navigate without a mouse. The Library
  page scans with `S`.
- **User profiles** (D-086). Display name, bio, and an avatar: upload a
  picture or pick one of eight mascots (Settings → Profile). Avatars show
  in the navigation and account menu. The "online" badge is gone.
- **Keyboard-complete reader.** The end-of-volume card traps focus on
  "Next"; `N`/`P` or `]`/`[` change volume, `B` goes back, `G` jumps to
  a page, `M` switches paged/strip, `W` toggles fit (comics), `?` lists
  every key.
- **Manga and comics** (D-085). New `manga` and `comic` library types scan
  `cbz`/`cbr`/`cb7` archives; a comic identifier reads series, volume,
  chapter/issue and year from filenames and folders. `lain.comic.pages@1`
  indexes archive pages and the gateway streams them
  (`/api/items/{id}/pages`), with covers from the first portrait page.
  The web reader (`/read/{id}`) is one engine with two adaptations: manga
  reads right to left with spreads and a webtoon strip, comics read left
  to right with fit, zoom and continuous scroll. Progress is saved per
  page and resumes where you left off. `cbr`/`cb7` need `bsdtar` or `7z`.
- **External list linking with AniList** (D-078..D-081). The lain list is
  a new per-user tracking domain (`lain.list.read@1`, `lain.list.write@1`,
  `lain.list.account@1` on bbolt buckets `list`/`list_accounts`):
  standalone entries for anime, manga, comics, movies and series that
  exist without library files, separate from catalog identity and
  playback progress. Users link an external platform from Settings →
  Account; `lain.listlink@1` is the ordered-many connector surface and
  `lain-listlink-anilist` implements AniList's authorization-code flow —
  authorize URL, code exchange, viewer lookup and an authenticated
  `MediaListCollection` fetch for ANIME and MANGA — plus the zero-setup
  **auth-pin** path (D-083): `GET /api/me/links/{platform}/pin` serves
  the official Lain app's authorize URL and `POST
  /api/me/links/{platform}/code` exchanges the pasted code through the
  embedded app credentials, normalizing a pasted URL or fragment.
  OAuth state is an
  HMAC-signed, expiring value bound to the local user id; tokens live
  server-side and never enter payloads or logs. Sync is
  remote-authoritative for platform-owned entries (upsert + remove
  missing) and runs on connect, on `POST /api/me/links/{platform}/sync`,
  and on a ~6h server ticker (`--list-sync=0`/`LAIN_LIST_SYNC=0`
  disables). A dead token answers `token-invalid`/`token-expired` and
  marks the link for reconnection while imported entries stay visible;
  unlinking deletes the token and the platform's entries. The operator
  registers one AniList OAuth app at the new admin page
  Settings → Integrations (`GET/PUT
  /api/admin/settings/integrations`, capability
  `lain.settings.integrations@1`), which prints the exact callback URL;
  `LAIN_ANILIST_CLIENT_ID`/`--anilist-client-id` and
  `LAIN_ANILIST_CLIENT_SECRET` (env-only) seed an empty configuration.
  The web UI gains a **My list** page (type/status filters, remote
  covers) and a Connected accounts section in Account settings.
- Browser direct play is decided by what the client says it can decode
  (D-058). `GET /api/items/{id}/playback` accepts an optional `caps`
  token list (`mkv`, `mkv/h264`, …) that the web client fills by probing
  a fixed matrix once per page load with `decodingInfo` **and**
  `canPlayType`; the planner requires the container *and* every track a
  player would select to be covered, so a Matroska file a real browser
  can decode is played as it is — no session, no encoder — and a seek
  becomes a Range request into the same file. An absent `caps` keeps the
  pre-D-058 rules exactly; tokens outside the server's vocabulary are
  dropped; and a `direct` plan is on probation until the first decoded
  picture, with a transcode fallback that says why instead of leaving a
  black screen.
- The web UI is title-centric. `/item/{id}` is now the anime/series/movie
  page — hero, poster rail, season filter and an episode grid — while a
  single-file title (a movie or a lone special) keeps the plain item
  page. `/player/{id}` becomes the watch page: the video with the title's
  episodes listed beside it, and the route follows the id param instead
  of loading once, so switching episodes cannot leave the previous
  episode's state or its marker on screen. Backed by a new additive read,
  `GET /api/catalog/{id}/episodes`, returning every file of the same
  title in watch order; the catalog owns the grouping rule (D-056).
- On-the-fly subtitle extraction (D-047): `GET
  /api/items/{id}/subtitles?stream=N` serves WebVTT for a chosen track
  of a **directly played** file, cached like any other derivative, so
  subtitles no longer require a transcode. A new
  `allow_subtitle_extraction` server setting (absent = on) turns it off
  with a stable `forbidden`.
- Per-stream stream-copy permissions (`allow_video_stream_copy`,
  `allow_audio_stream_copy`), mirroring Jellyfin's `PlaybackInfo`
  flags: an explicit false forces a re-encode of that stream, and the
  status now reports the outcome as `video_direct`/`audio_direct`.
- `deinterlace_method` (`yadif` or `bwdif`, with a visible fallback to
  `yadif` when the build lacks bwdif).
- Jellyfin-parity advanced transcode configuration (D-046): transcoding
  thread count, max muxing queue size, per-codec preset and CRF
  (H.264/H.265/AV1), audio VBR, downmix gain and stereo algorithm,
  deinterlace double rate, burn-in font fallback directory and family,
  per-codec 10-bit hardware-decode toggles, a server-wide remote client
  bitrate limit, and a configurable transcoding temporary path that
  relocates artifacts to `<path>/lain-transcode/<session>/` while the
  JSON index stays with the data dir.
- A configurable VA-API render node (`hardware_device`, default
  `/dev/dri/renderD128`), used by both the capability probe and the
  decode/encode `-hwaccel_device` argument, with a field in the
  Settings → Playback hardware section.
- Rockchip `rkmpp` joins the hardware backends, so the list matches
  Jellyfin's documented acceleration methods (VA-API, NVENC, QSV, AMF,
  V4L2, VideoToolbox, RKMPP). It is probed like every other backend and
  stays unavailable where the local ffmpeg lacks it.
- Per-user simultaneous stream limit (`max_streams`) enforced with a
  stable `too-many-streams` / HTTP 429, and a per-user default subtitle
  mode; both join the existing allow-video/audio-transcode, allow-remux
  and max-bitrate limits, and the admin session list now names the
  owning account.
- Live transcode metrics in the session status (`fps`,
  `output_bitrate_kbps`) read from ffmpeg's progress stream, shown in
  the Settings → Playback session list.
- Full browser transcode pipeline (`lain.playback.transcode@3`,
  composition v5, D-042..D-045): per-session options (quality ladder,
  resolution/bitrate caps, H.264/HEVC/AV1 output, audio codec and
  track, subtitle track and mode, delivery) and pipeline facts in the
  status (encoder actually used, hardware backend, software-fallback
  reason, why the transcode is needed, `playable` for HLS).
- HLS delivery with a server-owned `EVENT` playlist
  (`GET /api/items/{id}/transcode/hls/{file}`), playable while ffmpeg
  still runs, alongside the retained progressive MP4. The segment
  container is selectable (`fmp4` CMAF or `mpegts`). Throttling
  (SIGSTOP/SIGCONT against the client's fetched segment), optional
  segment deletion with a keep window, and idle-session cleanup.
- Hardware acceleration, opt-in only: real probe of the configured
  backend (VAAPI/NVENC/QSV/AMF/V4L2/VideoToolbox/RKMPP), per-codec
  hardware decoding, QSV low-power encoding, a configurable VA-API
  render node, and a visible software fallback when the probe or the
  run fails — never silent switching (D-031).
- HDR tone mapping (`zscale`+`tonemap`, libplacebo BT.2390 when its
  probe passes, with a visible downgrade to `hable` otherwise),
  auto-deinterlacing, quality-based scaling, audio downmix, and
  subtitle burn-in for image and styled text tracks.
- Operator surface: `GET/PUT /api/admin/settings/transcode` (validated,
  persisted in the bbolt `meta` bucket, applied to new sessions without
  a restart) returning the probed capabilities, `GET
  /api/admin/transcodes` plus `DELETE /api/admin/transcodes/{session}`
  for live sessions, `GET /api/playback/options` for the player ladder,
  and a transcode section in `lain doctor`.
- Per-user playback limits on the account: allow video transcoding,
  audio transcoding and remuxing, plus a maximum output bitrate; the
  gateway resolves them into the session and the plugin refuses what
  the user may not have (`forbidden`).
- Web: a Settings → Playback tab covering the whole policy (delivery,
  throttling, segment deletion, encoder preset/CRF per codec, codec
  permissions, quality ladder editor, hardware backend, decode codecs
  and 10-bit toggles, tone mapping, audio VBR/downmix/font fallback,
  thread count, muxing queue, cache/queue/concurrency, transcoding temp
  path, ffmpeg paths) with a live session list, a quality menu in the
  player with the transcode reasons shown, and per-user limits in the
  Users page. HLS playback uses `hls.js` (web-only dependency; D-043).
- Asynchronous browser transcoding (`lain.playback.transcode@2`,
  composition v3): `POST /api/items/{id}/transcode` starts or joins an
  idempotent session, `GET .../transcode/status?session=` polls
  `idle|queued|running|ready|failed`, and playback reports the opaque
  session plus profile/state without starting work. One global worker,
  bounded queue, 202 + `Retry-After` while pending; the synchronous
  `@1` endpoint stays compatible.
- Bounded transcode cache: plugin-private 20 GiB LRU (configurable via
  `--transcode-cache-size` / `LAIN_TRANSCODE_CACHE_SIZE`, or the admin
  UI), atomic sidecars, startup/admission/completion cleanup, stale and
  abandoned temporary removal.
- Probe-driven browser compatibility (`lain.media.probe@1`,
  composition v4): ffprobe stream inspection replaces extension
  guesses; single-audio selection, WebVTT subtitle sidecars
  (`GET /api/items/{id}/subtitles?session=`) with player track
  pickers, and explicit HDR refusal until a tone-map profile exists.
- Transcode sessions can begin at a source position (`start_sec`): a
  resume or a seek now seeks the input instead of re-encoding from the
  beginning, and the player offsets its clock and progress accordingly.

### Fixed
- The web player wrote "0:00, not finished" over a finished title that
  was only opened; it now writes nothing until the picture has played.
- On Android, the next episode kept its picture and gestures on the
  released player; it now shows and seeks.
- The full-screen web player stopped 10 px short of the window.
- The player's seek bar only ever reached as far as ffmpeg had encoded.
  An HLS session is an `EVENT` playlist listing just the produced
  segments, so under MSE the element's duration — and therefore
  `seekable` — was the produced edge: a 24-minute episode showed a 0:37
  timeline and refused a drag to minute 13. The plan response now carries
  the probed media length (`duration_sec`, the same fact Jellyfin's
  `PlaybackInfo` exposes as `RunTimeTicks`) and the player hands it to
  hls.js as a MediaSource duration override, so the bar spans the episode
  and progress is reported against the real length instead of the encoded
  prefix. A seek past what the encoder has written rebuilds the session
  at that position through `start_sec`, so it starts playing there rather
  than waiting for the encoder to arrive; a seek inside the produced range
  stays an ordinary, instant element seek. The buffered region is now
  drawn where it actually is, which a resumed or re-seeked session starts
  partway into. Found by the user watching a real episode.
- HLS playback in a real browser: the served playlist now signs every
  media URI with the session and the caller's token. A relative URI in
  an m3u8 drops the playlist's query string, so `hls.js` and native
  Safari fetched `init.mp4` and the segments unauthenticated and every
  request returned 401 — HLS never started. The signed URIs keep the
  existing `?token=` model and also work for native HLS. Covered by a Go
  regression test plus real-Chromium steps that assert the HLS playlist,
  an fMP4 segment and advancing playback, the player quality menu
  rebuilding the session, the retained progressive MP4 with Range,
  on-the-fly subtitle extraction rendering real cues during direct play,
  the advanced Settings → Playback UI, and the per-user playback limits
  dialog.
- Enabling hardware acceleration **without** hardware encoding did
  nothing: the plan reported hardware decode but `ffmpeg` received no
  `-hwaccel`, so the decode half stayed on the CPU. The decode hwaccel is
  now emitted from the configured backend, the runtime software fallback
  covers decode-only failures, and the session status names the backend
  actually in use.
- A configured transcoding temp path resolved to a doubled
  `lain-transcode/lain-transcode/<session>` directory when a finished
  session was reopened from its sidecar: `artifact_root` already carried
  the owned namespace and the resolver appended it again. Both sides now
  agree, and the previously-skipping round-trip test asserts it.
- The transcode test helper `makeMKV` placed `-pix_fmt` before a second
  `-i`, so `ffmpeg` rejected it and `TestPrepareTranscodesIncompatibleStreams`
  silently skipped; inputs now precede output options and the test runs.
- A VA-API session that also burns in an image subtitle kept its
  `hwupload`/`format=nv12` chain in the software retry: `softwareFallback`
  cleaned `filters` but not the burn-in `complexFilter`, so a failed
  hardware attempt failed again instead of falling back. The upload tokens
  are now stripped from both. Found by an adversarial subagent test.
- The requested bitrate cap was ignored whenever the plan chose a copy, so
  a web-safe source above the cap was remuxed at full bitrate while the
  status reported the cap; a cap below the probed source bitrate now
  forces a re-encode.
- An HLS session extracted its WebVTT sidecar only after the whole file
  had been produced, so it never offered a subtitle track while running
  (and the endpoint refused any session that was not ready). The sidecar
  is now extracted before the encode and served as soon as it exists.
- A source that already matched the requested MP4 audio codec was
  re-encoded unless the codec was AAC, and an unknown `audio_codec`
  reached the `ffmpeg` argv verbatim; matching codecs are copied and
  unknown ones are rejected.
- The ready-cache LRU could delete a fully produced HLS session while a
  client was still fetching its segments. A session accessed in the last
  minute is no longer an eviction victim (the overage is logged instead).
- `nightmode` downmixing applied its 5.1 coefficient matrix to any
  surround source; it now requires a real 5.1 source and says so in the
  session reasons, falling back to ffmpeg's layout-aware downmix.
- The reason a copy was refused by a client-side flag was overwritten by
  the later reason pass, so the status hid a cause it had recorded.
- The served HLS playlist dropped the `#EXT-X-DISCONTINUITY` ffmpeg
  wrote, and the parser attributed it to the segment *before* it (HLS
  puts the tag before the segment that starts the new timeline). A
  client could therefore stitch across a timestamp jump. The tag is now
  attributed to the following segment and re-emitted between segments.
- `position` was a high-water mark, so after a rewind ffmpeg kept racing
  ahead and segment deletion removed the very segments the client was
  about to re-fetch. The latest report now wins, matching Jellyfin's use
  of the reported playback position; an out-of-order report only makes
  the position older, so the change errs toward pausing sooner and
  keeping more segments, never toward deleting or racing ahead.
- An H.264/MP3 MP4 was transcoded even though browsers decode MP3 in
  MP4 as reliably as AAC. MP3 now direct-plays; AC-3/E-AC-3 stay
  excluded (Chromium on Linux cannot decode them).

## [0.2.0] - 2026-09-11

### Added
- Stable catalog identity across moves and renames: rescans reconcile
  by logical fingerprint and adopt the existing ID onto the new path,
  recording the previous path in `aliases`. Progress and enrichments
  survive moves untouched.
- Catalog export format version 2 (import accepts versions 1 and 2);
  scan stats report a `migrated` count.
- Installer: desktop launcher with icon and Lain identity, terminal
  setup wizard, and tiered uninstall.

### Fixed
- Installer media prompt keeps its default instead of losing it.

## [0.1.0] - 2026-09-10

- Initial public release: Go gateway with plugin composition, Bolt
  catalog, JWT auth, direct-play plans, metadata providers, library
  scanning, Docker image, and web administration.
