# Developer taste — lain server

Edit this freely. The advisor treats it as authoritative over model opinion and web research.

## Principles

- Prefer the standard library; a build measured in seconds matters more than elegance.
- Use small slices and explicit contracts; do not bundle unrelated work opportunistically.
- Errors are explicit and traceable; no silent failure and no panic on bad input.
- Preserve boundaries: plugins do not touch HTTP, media does not enter plugin calls, and catalog remains separate from userstate.

## Concrete preferences

- Go uses short, direct names without premature abstraction or generics where a concrete type is sufficient.
- Keep HTTP handlers thin, use standard-library method patterns, and avoid magical middleware.
- Test behavior first with `go test ./...`; fixtures use fictional placeholders such as `[Fansub-A]` and `tracker-example`.
- Repository-authored prose is English. Other languages belong only in explicit localization or translated-document variants.
- Use short imperative commit subjects.

## Never decide without the user (examples)

- Add or remove a dependency, accept cgo, or replace storage or authentication.
- Change a public API contract or begin a reserved slice such as transcode, metadata daemon, or Matrix.
- Make any change that breaks desktop-client compatibility.

## Open

- <!-- Example: logging, port, or CLI layout preferences. -->
