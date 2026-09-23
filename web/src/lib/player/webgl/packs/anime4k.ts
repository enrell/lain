import clamp from './Anime4K_Clamp_Highlights.glsl?raw';
import restoreS from './Anime4K_Restore_CNN_S.glsl?raw';
import restoreM from './Anime4K_Restore_CNN_M.glsl?raw';
import upscaleS from './Anime4K_Upscale_CNN_x2_S.glsl?raw';
import upscaleM from './Anime4K_Upscale_CNN_x2_M.glsl?raw';
import deblur from './Anime4K_Deblur_DoG.glsl?raw';
import darken from './Anime4K_Darken_VeryFast.glsl?raw';
import downscale2 from './Anime4K_AutoDownscalePre_x2.glsl?raw';
import downscale4 from './Anime4K_AutoDownscalePre_x4.glsl?raw';
import type { EffectPreset } from '../../effects-policy';

export interface HookPass {
	description: string;
	binds: string[];
	save: string;
	width?: string;
	height?: string;
	when?: string;
	source: string;
}

function parse(source: string): HookPass[] {
	return source.split(/(?=^\/\/!DESC )/m).slice(1).map((section) => {
		const directives = new Map<string, string[]>();
		const code: string[] = [];
		for (const line of section.split(/\r?\n/)) {
			const match = /^\/\/!(\w+)\s+(.*)$/.exec(line);
			if (match) directives.set(match[1], [...(directives.get(match[1]) ?? []), match[2]]);
			else code.push(line);
		}
		return {
			description: directives.get('DESC')?.[0] ?? 'Anime4K pass',
			binds: directives.get('BIND') ?? [],
			save: directives.get('SAVE')?.[0] ?? 'MAIN',
			width: directives.get('WIDTH')?.[0],
			height: directives.get('HEIGHT')?.[0],
			when: directives.get('WHEN')?.[0],
			source: code.join('\n')
		};
	});
}

const files = { clamp, restoreS, restoreM, upscaleS, upscaleM, deblur, darken, downscale2, downscale4 };
const parsed = Object.fromEntries(Object.entries(files).map(([name, source]) => [name, parse(source)])) as Record<keyof typeof files, HookPass[]>;

// Fragment equivalent of the existing WebGPU DoG x2 pack. It is separate
// from the mpv Ctrl+1/Ctrl+2/Lite hook chains above.
const dogX2: HookPass[] = [
	{
		description: 'Anime4K DoG luminance', binds: ['MAIN'], save: 'LINELUMA',
		source: 'vec4 hook() { return vec4(dot(MAIN_tex(MAIN_pos), vec4(0.299, 0.587, 0.114, 0.0)), 0.0, 0.0, 1.0); }'
	},
	{
		description: 'Anime4K DoG Gaussian X', binds: ['LINELUMA'], save: 'MMKERNEL',
		source: `vec4 hook() {
  vec2 d = vec2(LINELUMA_pt.x, 0.0);
  float c = LINELUMA_tex(LINELUMA_pos).r;
  float l = LINELUMA_tex(LINELUMA_pos - d).r;
  float r = LINELUMA_tex(LINELUMA_pos + d).r;
  float g = (LINELUMA_tex(LINELUMA_pos - d * 2.0).r + LINELUMA_tex(LINELUMA_pos + d * 2.0).r) * 0.06136;
  g += (l + r) * 0.24477 + c * 0.38774;
  return vec4(g, min(min(l, c), r), max(max(l, c), r), 1.0);
}`
	},
	{
		description: 'Anime4K DoG Gaussian Y', binds: ['MMKERNEL'], save: 'LINEKERNEL',
		source: `vec4 hook() {
  vec2 d = vec2(0.0, MMKERNEL_pt.y);
  vec3 c = MMKERNEL_tex(MMKERNEL_pos).rgb;
  vec3 a = MMKERNEL_tex(MMKERNEL_pos - d).rgb;
  vec3 b = MMKERNEL_tex(MMKERNEL_pos + d).rgb;
  float g = (MMKERNEL_tex(MMKERNEL_pos - d * 2.0).r + MMKERNEL_tex(MMKERNEL_pos + d * 2.0).r) * 0.06136;
  g += (a.r + b.r) * 0.24477 + c.r * 0.38774;
  return vec4(g, min(min(a.g, c.g), b.g), max(max(a.b, c.b), b.b), 1.0);
}`
	},
	{
		description: 'Anime4K DoG apply x2', binds: ['MAIN', 'LINELUMA', 'LINEKERNEL'], save: 'MAIN',
		width: 'NATIVE.w 2 *', height: 'NATIVE.h 2 *',
		source: `vec4 hook() {
  float lum = LINELUMA_tex(LINELUMA_pos).r;
  vec3 gauss = LINEKERNEL_tex(LINEKERNEL_pos).rgb;
  float correction = clamp((lum - gauss.r) * 0.8 + lum, gauss.g, gauss.b) - lum;
  return MAIN_tex(MAIN_pos) + vec4(correction, correction, correction, 0.0);
}`
	}
];

export function anime4kHooks(mode: EffectPreset): HookPass[] {
	if (mode === 'anime4k-dog-x2') return dogX2;
	const chain: (keyof typeof files)[] = mode === 'anime4k-a'
		? ['clamp', 'restoreM', 'upscaleM', 'downscale2', 'downscale4', 'upscaleS']
		: mode === 'anime4k-aa'
			? ['clamp', 'restoreM', 'upscaleM', 'restoreS', 'downscale2', 'downscale4', 'upscaleS']
			: mode === 'anime4k-lite'
				? ['clamp', 'restoreS', 'deblur', 'darken']
				: [];
	return chain.flatMap((name) => parsed[name]);
}
