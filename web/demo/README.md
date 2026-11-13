# The live demo

The site (`site/`, GitHub Pages) shows the web client itself, not pictures
of it: the client is built to run under `site/demo/` with no server, and a
service worker answers its API from a recorded snapshot.

| File | What it does |
|---|---|
| `build.mjs` | Builds the client with `LAIN_DEMO=1`: hash routes under `/lain/demo`, the player replaced by a notice. |
| `sw.js` | The demo's "server": answers `/api/*` from `snapshot/`; writes answer "ok" and change nothing. |
| `assemble.mjs` | `web/build` + `sw.js` + `snapshot/` into a folder a static host can serve. |
| `serve.mjs` | Serves the build under its base path; with `--record`, proxies a real server read-only and records its answers. |
| `record.mjs` | Walks the client in a headless browser so `serve.mjs --record` captures what it asks for. |
| `snapshot/` | The recorded answers (catalog, pictures, comic pages). Committed, and published as they are. |

The Pages workflow runs `build.mjs` and `assemble.mjs`. To record a new
snapshot, point `serve.mjs --record demo/snapshot` at a server that holds
only what may be published, then run `record.mjs` (see its header).

## What the snapshot shows, and its licences

- Posters of the Blender Studio open films (Big Buck Bunny, Sintel, Tears of
  Steel, Cosmos Laundromat, Elephants Dream, Sprite Fright, Charge):
  © Blender Foundation, [CC BY](https://studio.blender.org/films/), from
  Wikimedia Commons. The demo's "films" are stills, not the films.
- Pepper&Carrot, episodes 1 to 6, by David Revoy:
  [CC BY 4.0](https://www.peppercarrot.com/), pages resized.
- The shows are generated artwork made for lain's tests.
