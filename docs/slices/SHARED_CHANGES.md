# Shared changes — reading + downloads slice

Every edit this slice makes outside its own packages, so the parallel
SOCIAL slice can avoid or resolve conflicts. Own packages (no conflict
expected): `internal/downloads/`, `internal/plugins/comic/`,
`cmd/lain/download*.go`, `internal/gateway/downloads*.go`,
`web/src/routes/settings/downloads/`, `web/src/lib/api/downloads.ts`.

| File | Change | Conflict risk |
| --- | --- | --- |
| `internal/kv/kv.go` | New bucket constant `BDownloads` (`downloads`) added to the `Open` bucket list. | Low: one appended identifier; if social adds a bucket too, keep both in the list. |
| `internal/gateway/server.go` | `Server.downloads` field; manager built + started at the end of `NewWithOptions` (just before `s.routes()`), closed first in `Close`; `s.routesDownloads()` added after `s.routesList()`; `handleStream` honors `?download=1` (Content-Disposition attachment); `mime` import. | Medium: social will likely add a `routesX()` call and fields in the same spots — keep both lines. |
| `cmd/lain/main.go` | `case "download"` in the command switch and one usage line. | Low. |
| `cmd/lain/watch.go` | `playOneWithPlayer`: 3 lines after `streamURL` is built — use `localCopyPath(item.ID)` when an offline copy exists. | Low. |
| `cmd/lain/watch_queue.go` | `queueURL`: same 3-line local-copy check. | Low. |
