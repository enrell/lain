# lain for Android

The Android client (DESIGN-v2 §8.5, P5; decisions D-245..D-253): Kotlin +
Jetpack Compose for the UI, Media3/ExoPlayer for video, and the shared Rust
client core (`crates/lain-client`) through UniFFI bindings.

```
apps/android/
  lain-ffi/            the bindings crate (its own cargo workspace)
  app/                 the Compose app
  scripts/build-rust.sh  host lib + Kotlin bindings + cargo-ndk ABIs
  gradle/libs.versions.toml
```

## What it does

- **Connect and log in**: a server URL (plain HTTP for now, D-252) and an
  account; the session is kept for the next start.
- **Navigation from `/api/modules`** (D-250): Home, Library, Read, Search and
  Settings appear when their module is on; Downloads is the app's own.
- **Home**: a title-first hero, Continue, Recently added and one rail per
  library. **Library** and **Read** are poster grids; a **title page** lists
  episodes or volumes with progress and a Download action. **Search** asks the
  server, or offline matches titles in the last catalog snapshot.
- **Player** (Media3): an offline copy first, else the server's plan for
  `client=android` with the device's decode claim: direct play of
  `/api/items/{id}/stream`, or an HLS transcode session started at the resume
  point. Sidecar and embedded text subtitles are side-loaded as WebVTT.
  Progress saves every 5 s, on pause and on close through `ProgressSync`
  (queued when the server is unreachable); past 92 % counts as watched.
- **Reader** for comics and manga: pages from `/api/items/{id}/pages`, or from
  the offline CBZ copy on the device; right-to-left for manga; resumes at the
  saved page and saves every turn.
- **Offline copies** (D-248): downloads run in a foreground service within the
  device budget (Settings: budget, free-space floor, automatic eviction of
  watched copies); copies play and read without the server, and offline
  progress syncs when the event stream reconnects.
- **Themes** (`lain.ui.theme@1`, D-249): the desktop's two shipped themes,
  parsed by the Rust core; editable copies live in the app's
  `files/themes/<id>/theme.json`.

### Keys

Every action is focusable and runs on Enter. `Ctrl+1..9` jump to the
destinations, `/` opens Search, `Esc` goes back. Player: `Space` play/pause,
`←`/`→` seek 10 s, `B` burn typeset subtitles in. Reader: `←`/`→` in reading
order, `Space`/`PageDown` next, `PageUp` back, `Home`/`End`.

## ASS subtitles (D-247)

Media3 has no libass: ASS/SSA tracks show their text without typesetting
(positioning, fonts, karaoke and signs are lost), and the server's WebVTT
conversion drops styling too. When a file has an embedded ASS/SSA track, the
player says so and offers **Burn in** (`B`): the play restarts, from the
current position, as an HLS session the server transcodes with
`subtitle_mode: "burn"` for that stream. That needs ffmpeg on the server and
costs a transcode; an ASS *sidecar* cannot be burned in yet (the transcode
selection names embedded streams only). A libass-based player
(mpv-android, §8.5) is the later fallback and is not part of v1.

## Building

The SDK and NDK are **not** installed on the owner's machine (D-251); the
`android` CI job builds everything and uploads the debug APK as the
`lain-android-debug` artifact. On a machine with the Android SDK, NDK
(`ANDROID_NDK_HOME`), JDK 17+, `cargo-ndk` and the Rust targets
`aarch64-linux-android` and `x86_64-linux-android`:

```sh
apps/android/scripts/build-rust.sh            # libs + bindings (release libs)
cd apps/android && ./gradlew :app:assembleDebug
```

Only the Rust side builds and tests without Android tooling:

```sh
cargo test --manifest-path apps/android/lain-ffi/Cargo.toml
apps/android/scripts/build-rust.sh --bindings-only   # Kotlin bindings only
```

JVM unit tests: `./gradlew :app:testDebugUnitTest`; add
`-PlainHostLib=<path to the host liblain_ffi.so>` to run the bindings smoke
test through JNA.

## Gaps (v1)

- No TLS (the client core has none yet); no mDNS or QR pairing.
- CBR/CB7 books read online only (their extractors are external tools).
- Theme fonts, icon overrides, wallpapers, blur and live reload are not
  applied; the platform's sans, serif and mono families stand in.
- No Chromecast/DLNA, Android TV leanback, picture-in-picture or
  background audio; no release signing.
