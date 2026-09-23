# Clients

Lain currently serves a browser UI, a Go CLI (`lain login` / `lain watch`),
and a separate Linux desktop client in `projects/lain-desktop`. The web UI
and CLI can launch mpv or VLC on the browser's computer through the optional
local player handler. The desktop app keeps its existing libmpv playback.
The web and CLI clients share a preferred playback language through the
authenticated user account (D-071). The default player is local to each
browser or CLI installation.

## Planned TUI

A terminal user interface is wanted as another Lain client for users who
prefer keyboard-driven browsing and playback. This is a product direction,
not an implementation plan. No TUI command, toolkit, language, packaging
format, or repository layout has been chosen yet. In particular, whether
the TUI is written in Go and whether the current repositories become a
server repository plus separate helper/client repositories remain open.
Decide those questions before starting the TUI slice; preserve the existing
server API and authentication boundary when planning it.

Mobile clients are deferred. The current client work targets web, CLI,
desktop, and the planned TUI.
