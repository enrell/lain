# Browser shader engine

The web player has an optional first-party shader presentation layer. WebGPU is
preferred by default; the Brave profile prefers WebGL2 for the bundled mpv
Anime4K hooks. The
browser still owns demuxing, decoding, buffering, seeking, audio/video sync and
audio output through the existing `<video>` element. The renderer imports each
decoded video frame, normalizes it into an internal RGB texture, runs an ordered
WGSL graph and presents the result on a canvas.

This is presentation, not transcoding. It does not create an encoded video,
move frames through JavaScript or change any server contract.

## Runtime boundary

`web/src/lib/player/webgpu/` contains five deliberately small pieces:

- `scheduler.ts` requests exactly one submission per decoded frame through
  `requestVideoFrameCallback`.
- `graph.ts` validates the ordered DAG, resolves pass dimensions, estimates
  pixel work and assigns safe reusable texture slots.
- `texture-pool.ts` keeps those slots alive across frames and only reallocates
  when the decoded dimensions change.
- `shaders.ts` owns the ingest, display and pass ABI WGSL.
- `engine.ts` compiles pipelines and records ingest, effect and display passes.

The engine uses `rgba16float` between passes. `GPUExternalTexture` is imported
again for every decoded frame, as required by WebGPU; it is sampled only by the
ingest pass. Every subsequent pass sees an ordinary `GPUTexture`.

The graph never reorders passes. Order is part of visual meaning. The planner
does expose `estimatedPixels`, so a preset author can compare `Restore ->
Upscale` with the more expensive `Upscale -> Restore` without the runtime
silently changing either pipeline.

## WGSL ABI

WGSL is the only native shader format. A pass supplies one function:

```wgsl
fn effect(uv: vec2f) -> vec4f {
  return sample_input(0u, uv);
}
```

The engine supplies:

```wgsl
struct ShaderParams {
  input_size: vec2f,
  output_size: vec2f,
  time_seconds: f32,
  frame_index: u32,
  values0: vec4f,
  values1: vec4f,
}

fn sample_input(index: u32, uv: vec2f) -> vec4f;
fn texel_size() -> vec2f;
```

A pass may name up to four earlier outputs and expose up to eight scalar
parameters. It declares its output scale and texture format in TypeScript, not
inside WGSL:

```ts
const restore: ShaderPass = {
  id: 'example.restore',
  label: 'Example restore',
  wgsl: restoreWGSL,
  scale: 1,
  params: [0.35]
};
```

Pass ids are unique. An omitted `inputs` list means the previous output; the
first pass receives `$source`. Explicit inputs may refer only to `$source` or
an earlier pass, which makes cycles and forward references invalid before any
GPU resource is allocated.

## Fallbacks

WebGPU is optional. The player keeps `<video>` as the visible path until the
first GPU frame has been submitted. When WebGPU has no compatible adapter or
initialization fails, the Mode A, Mode A+A and Lite presets can use WebGL2 and
the original Anime4K mpv GLSL hooks on the same decoded video frames. This
requires WebGL2 floating-point render targets. If neither GPU path works, the
native video remains available.

The browser renderer profile prefers WebGL2 in Brave and WebGPU elsewhere.
Brave is identified through its `navigator.brave.isBrave()` API, with the Brave
client-hint brand as a fallback; a generic Chromium user agent is not enough
to identify it. Each backend still checks its actual adapter/context and shader
support. Separate canvases allow the second backend to initialize even when
the first one acquired a different canvas context before failing. The selected
profile and active backend appear under Playback information. The profile does
not alter browser GPU flags or repair a broken native video compositor.

Before a GPU canvas replaces the video element, the player checks that the
browser exposes readable pixels from the decoded frame. Some browser/driver
configurations play audio and composite video normally but return blank frames
to canvas, WebGL and WebGPU. In that case the effect cannot process the video;
after five seconds of blank frames the player reports the limitation and keeps
the native video visible. `Off` never starts a shader renderer.

Native sidecar subtitles are rendered by the browser on the video element. The
player therefore pauses the WebGPU presentation layer while such a text track
is selected. Burned-in subtitles are already pixels and need no special path.

The no-op built-in pass runs through the production ABI. It intentionally does
not change the image and is used for the `Off` path while the renderer is
available.

## Shader packs and provenance

Anime4K is a shader pack, not an engine dependency. The first pack,
`ANIME4K_DOG_X2`, adapts the upstream four-pass DoG pipeline:

1. luminance extraction;
2. horizontal Gaussian blur with local luminance bounds;
3. vertical Gaussian blur with local luminance bounds;
4. bounded edge correction and 2x output.

It is available from the player's `Effects` selector and remains opt-in; `Off`
is the default. The source is isolated under `packs/`, and both upstream MIT
licenses are reproduced in `docs/THIRD_PARTY.md`.

Extending D-059/D-060 within D-066's player settings, the browser also offers
three presets modeled on this machine's mpv shader
chains. `Mode A` follows Ctrl+1 (Clamp Highlights, Restore CNN M, Upscale CNN
M, conditional AutoDownscalePre, Upscale CNN S). `Mode A+A` follows Ctrl+2 and
adds Restore CNN S after the first upscale. `Lite` combines Clamp Highlights,
Restore CNN S, Deblur DoG and Darken VeryFast, with no upscale. These presets
use the corresponding MIT-licensed compute WGSL from anime4k-wgpu; the
selected, generated pipeline descriptors are vendored in
`packs/anime4k-predefined.json`, without a runtime dependency. The optional
compute path requires WebGPU's `float32-filterable` adapter feature. If it is
missing or a shader fails, the browser's native video remains visible.

The mpv AutoDownscalePre stages depend on the actual display scale. The browser
path follows their 1.2x, 2x, 2.4x and 4x thresholds and resamples before a
second upscale when needed. Canvas composition and WebGPU sampling may still
produce slightly different pixels than mpv's GLSL renderer; the presets are
pipeline equivalents, not a bit-exact GLSL emulation.

The WebGL2 fallback uses the nine original MIT-licensed mpv hook files under
`web/src/lib/player/webgl/packs/`. Its small hook runner evaluates only the
directives used by these bundled files; it does not accept arbitrary shaders.
The files are included in the browser bundle, with no server-side processing.
The DoG x2 preset has the same four fragment stages in both backends.

`Settings → Video effects` saves an initial choice and ordered overrides in
browser storage under the signed-in account. A rule can match library type,
library ID, video stream index, and inclusive source-height bounds. Resolution
is the source video height, before any shader upscale. Precedence is stream,
resolution, library, library type, then the global default; the last matching
row wins ties. This is local to the current browser and does not change the
server or desktop client. The player can change the effect for its current
viewing session without modifying saved defaults. A fresh account remains Off.

The WebGPU graph accepts WGSL only. The WebGL2 fallback accepts only its
bundled Anime4K GLSL hooks. Neither runtime accepts user-supplied shader code.

## Verification

`graph.test.ts` fixes ordering, dimensions, work estimates, DAG validation and
texture reuse. `scheduler.test.ts` fixes the one-callback/one-frame contract.
The procedural browser probe uses a canvas-captured synthetic video so it can
compile the complete Anime4K DoG graph, import a real `GPUExternalTexture` and
present a 2x frame without depending on a media file or server state. It also
compiles and submits Mode A, A+A and Lite through the compute path, including
the conditional auto-downscale and 4x branches. The
normal product smoke launches Chromium with the GPU disabled and therefore
covers the native fallback. A product-level WebGPU run covers activation and
effect switching against the embedded web application.
