import { describe, expect, it } from 'vitest';
import { planShaderGraph, SOURCE_NODE } from '../graph';
import { ANIME4K_DOG_X2 } from './anime4k';

describe('Anime4K DoG x2 pack', () => {
	it('is a generic four-pass graph with one explicit 2x output', () => {
		expect(ANIME4K_DOG_X2.map((pass) => pass.id)).toEqual([
			'anime4k.dog.luma',
			'anime4k.dog.gaussian-x',
			'anime4k.dog.gaussian-y',
			'anime4k.dog.apply-2x'
		]);
		expect(ANIME4K_DOG_X2[1].inputs).toEqual([SOURCE_NODE, 'anime4k.dog.luma']);
		expect(ANIME4K_DOG_X2[3].inputs).toEqual([
			SOURCE_NODE,
			'anime4k.dog.luma',
			'anime4k.dog.gaussian-y'
		]);
		expect(ANIME4K_DOG_X2[3].scale).toBe(2);
	});

	it('plans three source-size passes before the four-times-larger apply pass', () => {
		const plan = planShaderGraph(ANIME4K_DOG_X2, { width: 640, height: 360 });
		expect(plan.passes.map((pass) => [pass.width, pass.height])).toEqual([
			[640, 360],
			[640, 360],
			[640, 360],
			[1280, 720]
		]);
		expect(plan.output.width).toBe(1280);
		expect(plan.output.height).toBe(720);
		expect(plan.estimatedPixels).toBe(7 * 640 * 360);
	});

	it('retains upstream provenance in every adapted shader', () => {
		for (const pass of ANIME4K_DOG_X2) {
			expect(pass.wgsl).toContain('Copyright (c) 2019-2021 bloc97');
			expect(pass.wgsl).toContain('adapted from anime4k-wgpu');
		}
	});
});
