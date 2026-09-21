import type { ShaderPass } from './graph';

const FULLSCREEN_VERTEX = /* wgsl */ `
struct VertexOutput {
  @builtin(position) position: vec4f,
  @location(0) uv: vec2f,
}

@vertex
fn vertex_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
  let positions = array<vec2f, 3>(
    vec2f(-1.0, -1.0),
    vec2f( 3.0, -1.0),
    vec2f(-1.0,  3.0),
  );
  let position = positions[vertex_index];
  var output: VertexOutput;
  output.position = vec4f(position, 0.0, 1.0);
  output.uv = position * vec2f(0.5, -0.5) + vec2f(0.5);
  return output;
}
`;

export const INGEST_SHADER = /* wgsl */ `
${FULLSCREEN_VERTEX}

@group(0) @binding(0) var frame_sampler: sampler;
@group(0) @binding(1) var frame_texture: texture_external;

@fragment
fn fragment_main(input: VertexOutput) -> @location(0) vec4f {
  return textureSampleBaseClampToEdge(frame_texture, frame_sampler, input.uv);
}
`;

export const DISPLAY_SHADER = /* wgsl */ `
${FULLSCREEN_VERTEX}

@group(0) @binding(0) var frame_sampler: sampler;
@group(0) @binding(1) var frame_texture: texture_2d<f32>;

@fragment
fn fragment_main(input: VertexOutput) -> @location(0) vec4f {
  return textureSample(frame_texture, frame_sampler, input.uv);
}
`;

/**
 * Fixed WGSL ABI. Four texture inputs keep the runtime generic enough for
 * branched restoration graphs without making each shader invent bindings.
 */
const EFFECT_ABI = /* wgsl */ `
${FULLSCREEN_VERTEX}

struct ShaderParams {
  input_size: vec2f,
  output_size: vec2f,
  time_seconds: f32,
  frame_index: u32,
  values0: vec4f,
  values1: vec4f,
}

@group(0) @binding(0) var effect_sampler: sampler;
@group(0) @binding(1) var effect_input0: texture_2d<f32>;
@group(0) @binding(2) var effect_input1: texture_2d<f32>;
@group(0) @binding(3) var effect_input2: texture_2d<f32>;
@group(0) @binding(4) var effect_input3: texture_2d<f32>;
@group(0) @binding(5) var<uniform> params: ShaderParams;

fn sample_input(index: u32, uv: vec2f) -> vec4f {
  switch index {
    case 1u: { return textureSample(effect_input1, effect_sampler, uv); }
    case 2u: { return textureSample(effect_input2, effect_sampler, uv); }
    case 3u: { return textureSample(effect_input3, effect_sampler, uv); }
    default: { return textureSample(effect_input0, effect_sampler, uv); }
  }
}

fn texel_size() -> vec2f {
  return 1.0 / params.input_size;
}
`;

export function effectShader(pass: ShaderPass): string {
	return /* wgsl */ `${EFFECT_ABI}

${pass.wgsl}

@fragment
fn fragment_main(input: VertexOutput) -> @location(0) vec4f {
  return effect(input.uv);
}
`;
}

/** Original no-op pack used to verify the ABI without changing the picture. */
export const PASSTHROUGH_PASS: ShaderPass = {
	id: 'builtin.passthrough',
	label: 'Pass through',
	wgsl: /* wgsl */ `
fn effect(uv: vec2f) -> vec4f {
  return sample_input(0u, uv);
}
`
};
