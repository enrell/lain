# WebGPU shader engine

The web player has an optional first-party WebGPU presentation layer. The
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
first GPU frame has been submitted. If WebGPU, a compatible adapter, shader
compilation, the canvas context or a later device submission fails, the canvas
is disabled and the native video remains available.

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

Do not add GLSL, HLSL, SPIR-V or mpv-hook translation to the runtime. Any such
conversion belongs in an offline tool and must still produce reviewed WGSL.

## Verification

`graph.test.ts` fixes ordering, dimensions, work estimates, DAG validation and
texture reuse. `scheduler.test.ts` fixes the one-callback/one-frame contract.
The procedural browser probe uses a canvas-captured synthetic video so it can
compile the complete Anime4K DoG graph, import a real `GPUExternalTexture` and
present a 2x frame without depending on a media file or server state. The
normal product smoke launches Chromium with the GPU disabled and therefore
covers the native fallback. A product-level WebGPU run covers activation and
effect switching against the embedded web application.
