import { describe, expect, it } from 'vitest';
import { SOURCE_NODE, planShaderGraph, type ShaderPass } from './graph';

const shader = 'fn effect(uv: vec2f) -> vec4f { return sample_input(0u, uv); }';

function pass(id: string, scale = 1): ShaderPass {
	return { id, label: id, wgsl: shader, scale };
}

describe('planShaderGraph', () => {
	it('plans a linear graph as reusable ping-pong textures', () => {
		const plan = planShaderGraph(
			[pass('restore'), pass('upscale', 2), pass('sharpen'), pass('grade')],
			{ width: 960, height: 540 }
		);

		expect(plan.passes.map((node) => [node.id, node.width, node.height])).toEqual([
			['restore', 960, 540],
			['upscale', 1920, 1080],
			['sharpen', 1920, 1080],
			['grade', 1920, 1080]
		]);
		expect(plan.sourceSlot).not.toBe(plan.passes[0].outputSlot);
		expect(plan.passes[1].outputSlot).not.toBe(plan.passes[2].outputSlot);
		expect(plan.passes[3].outputSlot).toBe(plan.passes[1].outputSlot);
		expect(plan.slots).toHaveLength(4);
	});

	it('keeps a branched input alive until its last consumer', () => {
		const plan = planShaderGraph(
			[
				pass('features'),
				{ ...pass('detail'), inputs: ['features'] },
				{ ...pass('merge'), inputs: [SOURCE_NODE, 'detail'] }
			],
			{ width: 1280, height: 720 }
		);

		const source = plan.sourceSlot;
		const detail = plan.passes[1].outputSlot;
		const merge = plan.passes[2].outputSlot;
		expect(merge).not.toBe(source);
		expect(merge).not.toBe(detail);
	});

	it('reports the pixel cost without silently reordering quality-sensitive passes', () => {
		const restoreFirst = planShaderGraph(
			[pass('restore'), pass('upscale', 2)],
			{ width: 1920, height: 1080 }
		);
		const upscaleFirst = planShaderGraph(
			[pass('upscale', 2), pass('restore')],
			{ width: 1920, height: 1080 }
		);

		expect(restoreFirst.passes.map((node) => node.id)).toEqual(['restore', 'upscale']);
		expect(upscaleFirst.passes.map((node) => node.id)).toEqual(['upscale', 'restore']);
		expect(restoreFirst.estimatedPixels).toBe(1920 * 1080 + 3840 * 2160);
		expect(upscaleFirst.estimatedPixels).toBe(2 * 3840 * 2160);
		expect(restoreFirst.estimatedPixels).toBeLessThan(upscaleFirst.estimatedPixels);
	});

	it('rejects forward references, duplicate ids and unsafe dimensions', () => {
		expect(() =>
			planShaderGraph([{ ...pass('late'), inputs: ['missing'] }], { width: 640, height: 360 })
		).toThrow(/unknown or forward input/);
		expect(() =>
			planShaderGraph([pass('same'), pass('same')], { width: 640, height: 360 })
		).toThrow(/duplicate pass id/);
		expect(() => planShaderGraph([pass('huge', 8)], { width: 4096, height: 2160 }, 8192)).toThrow(
			/exceeds the device limit/
		);
		expect(() =>
			planShaderGraph(
				[{ ...pass('wide'), inputs: [SOURCE_NODE, SOURCE_NODE, SOURCE_NODE, SOURCE_NODE, SOURCE_NODE] }],
				{ width: 640, height: 360 }
			)
		).toThrow(/ABI supports 4/);
		expect(() =>
			planShaderGraph([{ ...pass('many-params'), params: Array(9).fill(0) }], { width: 640, height: 360 })
		).toThrow(/ABI supports 8/);
	});
});
