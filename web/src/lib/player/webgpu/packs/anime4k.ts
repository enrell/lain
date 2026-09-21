import { SOURCE_NODE, type ShaderPass } from '../graph';

/*
 * Anime4K Upscale DoG x2
 * Copyright (c) 2019-2021 bloc97
 * WGSL port Copyright (c) 2026 SegaraRai
 *
 * Adapted from anime4k-wgpu's compute WGSL to Lain's fragment-pass ABI.
 * The algorithm and coefficients are unchanged; storage writes became render
 * targets and clamped texture sampling supplies the original edge behavior.
 * Both upstream works are MIT licensed; see docs/THIRD_PARTY.md.
 */
const PROVENANCE = `
// MIT-licensed Anime4K algorithm, Copyright (c) 2019-2021 bloc97.
// WGSL adapted from anime4k-wgpu, Copyright (c) 2026 SegaraRai.
`;

const luma: ShaderPass = {
	id: 'anime4k.dog.luma',
	label: 'Anime4K DoG — luminance',
	wgsl: /* wgsl */ `${PROVENANCE}
fn effect(uv: vec2f) -> vec4f {
  let color = sample_input(0u, uv);
  let luminance = dot(vec4f(0.299, 0.587, 0.114, 0.0), color);
  return vec4f(luminance, 0.0, 0.0, 1.0);
}
`
};

const gaussianX: ShaderPass = {
	id: 'anime4k.dog.gaussian-x',
	label: 'Anime4K DoG — horizontal Gaussian/min-max',
	inputs: [SOURCE_NODE, luma.id],
	wgsl: /* wgsl */ `${PROVENANCE}
fn luma_at(uv: vec2f) -> f32 {
  return sample_input(1u, uv).r;
}

fn effect(uv: vec2f) -> vec4f {
  let delta = vec2f(texel_size().x, 0.0);
  let center = luma_at(uv);
  let left = luma_at(uv - delta);
  let right = luma_at(uv + delta);
  var gaussian = (luma_at(uv - delta * 2.0) + luma_at(uv + delta * 2.0)) * 0.06136;
  gaussian += (left + right) * 0.24477;
  gaussian += center * 0.38774;
  return vec4f(gaussian, min(min(left, center), right), max(max(left, center), right), 1.0);
}
`
};

const gaussianY: ShaderPass = {
	id: 'anime4k.dog.gaussian-y',
	label: 'Anime4K DoG — vertical Gaussian/min-max',
	inputs: [gaussianX.id],
	wgsl: /* wgsl */ `${PROVENANCE}
fn gaussian_at(uv: vec2f) -> vec3f {
  return sample_input(0u, uv).rgb;
}

fn effect(uv: vec2f) -> vec4f {
  let delta = vec2f(0.0, texel_size().y);
  let center = gaussian_at(uv);
  let above = gaussian_at(uv - delta);
  let below = gaussian_at(uv + delta);
  var gaussian = (gaussian_at(uv - delta * 2.0).r + gaussian_at(uv + delta * 2.0).r) * 0.06136;
  gaussian += (above.r + below.r) * 0.24477;
  gaussian += center.r * 0.38774;
  let minimum = min(min(above.g, center.g), below.g);
  let maximum = max(max(above.b, center.b), below.b);
  return vec4f(gaussian, minimum, maximum, 1.0);
}
`
};

const apply: ShaderPass = {
	id: 'anime4k.dog.apply-2x',
	label: 'Anime4K DoG — apply ×2',
	inputs: [SOURCE_NODE, luma.id, gaussianY.id],
	scale: 2,
	params: [0.8],
	wgsl: /* wgsl */ `${PROVENANCE}
fn effect(uv: vec2f) -> vec4f {
  let luminance = sample_input(1u, uv).r;
  let gaussian = sample_input(2u, uv).rgb;
  let correction = clamp((luminance - gaussian.r) * params.values0.x + luminance, gaussian.g, gaussian.b) - luminance;
  return sample_input(0u, uv) + vec4f(correction, correction, correction, 0.0);
}
`
};

/** Conservative first Anime4K pack: the upstream four-pass DoG x2 pipeline. */
export const ANIME4K_DOG_X2: readonly ShaderPass[] = [luma, gaussianX, gaussianY, apply];
