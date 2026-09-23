import predefined from './anime4k-predefined.json';

// Generated WGSL and pipeline descriptors from SegaraRai/anime4k-wgpu
// (commit 526f5808); Anime4K and the WGSL port are MIT licensed.
// See docs/THIRD_PARTY.md. No runtime package or server processing is used.

type PipelineName = keyof typeof predefined;
export type Anime4KMode = 'anime4k-a' | 'anime4k-aa' | 'anime4k-lite';

const CHAINS: Record<Anime4KMode, readonly PipelineName[]> = {
	'anime4k-a': ['CLAMP_HIGHLIGHTS', 'RESTORE_CNN_M', 'UPSCALE_CNN_X2_M', 'UPSCALE_CNN_X2_S'],
	'anime4k-aa': ['CLAMP_HIGHLIGHTS', 'RESTORE_CNN_M', 'UPSCALE_CNN_X2_M', 'RESTORE_CNN_S', 'UPSCALE_CNN_X2_S'],
	'anime4k-lite': ['CLAMP_HIGHLIGHTS', 'RESTORE_CNN_S', 'DEBLUR_DOG', 'EFFECTS_DARKEN_VERYFAST']
};

interface Scale { numerator: number; denominator: number }
interface PhysicalTexture {
	id: number;
	components: number;
	scale_factor: [Scale, Scale];
	is_source: boolean;
}
interface Binding { binding: number; physical_id: number }
interface Pass {
	id: string;
	shader: string;
	compute_scale_factors: [number, number];
	input_textures: Binding[];
	output_textures: Binding[];
	samplers: { binding: number; filter_mode: 'nearest' | 'linear' }[];
}
interface Pipeline {
	physical_textures: PhysicalTexture[];
	passes: Pass[];
}
interface BoundPass {
	pipeline: GPUComputePipeline;
	bindGroup: GPUBindGroup;
	width: number;
	height: number;
}
interface BoundResample {
	pipeline: GPURenderPipeline;
	bindGroup: GPUBindGroup;
	output: GPUTexture;
}

const PIPELINES = predefined as unknown as Record<PipelineName, Pipeline>;
const GPU_COMPUTE_STAGE = 0x4;
const GPU_STORAGE_BINDING = 0x8;
const GPU_TEXTURE_BINDING = 0x4;

function format(components: number): GPUTextureFormat {
	return components === 1 ? 'r32float' : components === 2 ? 'rg32float' : 'rgba32float';
}

/** The mpv chain's CNN and auxiliary compute passes, isolated from the generic WGSL graph. */
export class Anime4KComputePack {
	private readonly compiled = new Map<PipelineName, GPUComputePipeline[]>();
	private readonly samplers = new Map<string, GPUSampler>();
	private textures: GPUTexture[] = [];
	private passes: (BoundPass | BoundResample)[] = [];
	private resamplePipeline: GPURenderPipeline | null = null;
	private source: GPUTexture | null = null;
	private sourceWidth = 0;
	private sourceHeight = 0;
	private targetWidth = 0;
	private targetHeight = 0;
	private output: GPUTexture | null = null;

	private constructor(private readonly device: GPUDevice, private readonly mode: Anime4KMode) {
		for (const filter of ['nearest', 'linear'] as const) {
			this.samplers.set(filter, device.createSampler({
				magFilter: filter, minFilter: filter, addressModeU: 'clamp-to-edge', addressModeV: 'clamp-to-edge'
			}));
		}
	}

	static async create(device: GPUDevice, mode: Anime4KMode): Promise<Anime4KComputePack> {
		const pack = new Anime4KComputePack(device, mode);
		const resampleModule = device.createShaderModule({ label: 'Anime4K auto downscale', code: /* wgsl */ `
@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var linear_sampler: sampler;
struct VertexOut { @builtin(position) position: vec4f, @location(0) uv: vec2f }
@vertex fn vertex_main(@builtin(vertex_index) index: u32) -> VertexOut {
  let positions = array<vec2f, 3>(vec2f(-1.0, -1.0), vec2f(3.0, -1.0), vec2f(-1.0, 3.0));
  let position = positions[index];
  var result: VertexOut;
  result.position = vec4f(position, 0.0, 1.0);
  result.uv = position * vec2f(0.5, -0.5) + vec2f(0.5);
  return result;
}
@fragment fn fragment_main(input: VertexOut) -> @location(0) vec4f {
  return textureSample(source, linear_sampler, input.uv);
}` });
		const resampleInfo = await resampleModule.getCompilationInfo();
		if (resampleInfo.messages.some((message) => message.type === 'error')) {
			throw new Error('Anime4K auto downscale shader failed to compile');
		}
		pack.resamplePipeline = await device.createRenderPipelineAsync({
			label: 'Anime4K auto downscale', layout: 'auto',
			vertex: { module: resampleModule, entryPoint: 'vertex_main' },
			fragment: { module: resampleModule, entryPoint: 'fragment_main', targets: [{ format: 'rgba32float' }] },
			primitive: { topology: 'triangle-list' }
		});
		for (const name of CHAINS[mode]) {
			const pipelines: GPUComputePipeline[] = [];
			for (const pass of PIPELINES[name].passes) {
				const module = device.createShaderModule({ label: `${name}: ${pass.id}`, code: pass.shader });
				const info = await module.getCompilationInfo();
				const errors = info.messages.filter((message) => message.type === 'error');
				if (errors.length) throw new Error(`${name}/${pass.id}: ${errors.map((message) => message.message).join('; ')}`);
				const entries: GPUBindGroupLayoutEntry[] = [
					...pass.input_textures.map((input) => ({ binding: input.binding, visibility: GPU_COMPUTE_STAGE, texture: { sampleType: 'float' as const } })),
					...pass.output_textures.map((output) => {
						const texture = PIPELINES[name].physical_textures.find((candidate) => candidate.id === output.physical_id);
						if (!texture) throw new Error(`${name}/${pass.id}: missing output texture`);
						return { binding: output.binding, visibility: GPU_COMPUTE_STAGE, storageTexture: { access: 'write-only' as const, format: format(texture.components) } };
					}),
					...pass.samplers.map((sampler) => ({ binding: sampler.binding, visibility: GPU_COMPUTE_STAGE, sampler: { type: 'filtering' as const } }))
				].sort((a, b) => a.binding - b.binding);
				const layout = device.createBindGroupLayout({ label: `${name}/${pass.id}`, entries });
				pipelines.push(await device.createComputePipelineAsync({
					label: `${name}/${pass.id}`,
					layout: device.createPipelineLayout({ bindGroupLayouts: [layout] }),
					compute: { module, entryPoint: 'main' }
				}));
			}
			pack.compiled.set(name, pipelines);
		}
		return pack;
	}

	/** mpv's x2 passes run only when display scale is above 1.2. */
	private chain(width: number, height: number, targetWidth: number, targetHeight: number): PipelineName[] {
		if (this.mode === 'anime4k-lite') return [...CHAINS[this.mode]];
		const scale = Math.min(targetWidth / width, targetHeight / height);
		const first = scale > 1.2;
		const second = scale > 2.4;
		return CHAINS[this.mode].filter((name) =>
			name !== 'UPSCALE_CNN_X2_M' && name !== 'UPSCALE_CNN_X2_S' ||
			(name === 'UPSCALE_CNN_X2_M' && first) || (name === 'UPSCALE_CNN_X2_S' && second)
		);
	}

	private resample(input: GPUTexture, width: number, height: number): GPUTexture {
		if (!this.resamplePipeline || width < 1 || height < 1 ||
			width > this.device.limits.maxTextureDimension2D || height > this.device.limits.maxTextureDimension2D) {
			throw new Error(`Anime4K auto downscale size ${width}x${height} exceeds device limits`);
		}
		const output = this.device.createTexture({
			label: 'Anime4K auto downscale output', size: { width, height }, format: 'rgba32float',
			usage: GPU_TEXTURE_BINDING | 0x10
		});
		this.textures.push(output);
		this.passes.push({
			pipeline: this.resamplePipeline,
			bindGroup: this.device.createBindGroup({
				layout: this.resamplePipeline.getBindGroupLayout(0),
				entries: [
					{ binding: 0, resource: input.createView() },
					{ binding: 1, resource: this.samplers.get('linear')! }
				]
			}),
			output
		});
		return output;
	}

	configure(source: GPUTexture, targetWidth: number, targetHeight: number): GPUTexture {
		if (this.source === source && this.sourceWidth === source.width && this.sourceHeight === source.height &&
			this.targetWidth === targetWidth && this.targetHeight === targetHeight && this.output) return this.output;
		this.release();
		this.source = source;
		this.sourceWidth = source.width;
		this.sourceHeight = source.height;
		this.targetWidth = targetWidth;
		this.targetHeight = targetHeight;
		let current = source;
		const scale = Math.min(targetWidth / source.width, targetHeight / source.height);
		for (const name of this.chain(source.width, source.height, targetWidth, targetHeight)) {
			// mpv's AutoDownscalePre_x4 reduces the first 2x result to
			// half the display size before the final small CNN upscaler.
			if (name === 'UPSCALE_CNN_X2_S' && scale > 2.4 && scale < 4) {
				current = this.resample(current, Math.floor(targetWidth / 2), Math.floor(targetHeight / 2));
			}
			const pipeline = PIPELINES[name];
			const physical = new Map<number, GPUTexture>();
			for (const texture of pipeline.physical_textures) {
				if (texture.is_source) {
					physical.set(texture.id, current);
					continue;
				}
				const width = Math.floor(current.width * texture.scale_factor[0].numerator / texture.scale_factor[0].denominator);
				const height = Math.floor(current.height * texture.scale_factor[1].numerator / texture.scale_factor[1].denominator);
				if (width < 1 || height < 1 || width > this.device.limits.maxTextureDimension2D || height > this.device.limits.maxTextureDimension2D) {
					throw new Error(`${name}: texture size ${width}x${height} exceeds device limits`);
				}
				const created = this.device.createTexture({
					label: `${name} texture ${texture.id}`, size: { width, height }, format: format(texture.components),
					usage: GPU_STORAGE_BINDING | GPU_TEXTURE_BINDING
				});
				this.textures.push(created);
				physical.set(texture.id, created);
			}
			pipeline.passes.forEach((pass, index) => {
				const compiled = this.compiled.get(name)?.[index];
				if (!compiled) throw new Error(`${name}/${pass.id}: pipeline not compiled`);
				const entries: GPUBindGroupEntry[] = [
					...pass.input_textures.map((input) => ({ binding: input.binding, resource: physical.get(input.physical_id)!.createView() })),
					...pass.output_textures.map((output) => ({ binding: output.binding, resource: physical.get(output.physical_id)!.createView() })),
					...pass.samplers.map((sampler) => ({ binding: sampler.binding, resource: this.samplers.get(sampler.filter_mode)! }))
				].sort((a, b) => a.binding - b.binding);
				this.passes.push({
					pipeline: compiled,
					bindGroup: this.device.createBindGroup({ layout: compiled.getBindGroupLayout(0), entries }),
					width: Math.floor(current.width * pass.compute_scale_factors[0]),
					height: Math.floor(current.height * pass.compute_scale_factors[1])
				});
			});
			current = physical.get(pipeline.passes.at(-1)!.output_textures[0].physical_id)!;
		}
		// mpv's AutoDownscalePre_x2 avoids keeping an oversized first CNN
		// output when the display only needs between 1.2x and 2x.
		if (this.mode !== 'anime4k-lite' && scale > 1.2 && scale < 2) {
			current = this.resample(current, targetWidth, targetHeight);
		}
		this.output = current;
		return current;
	}

	encode(encoder: GPUCommandEncoder): void {
		for (const pass of this.passes) {
			if ('output' in pass) {
				const render = encoder.beginRenderPass({ colorAttachments: [{
					view: pass.output.createView(), loadOp: 'clear', storeOp: 'store',
					clearValue: { r: 0, g: 0, b: 0, a: 1 }
				}] });
				render.setPipeline(pass.pipeline);
				render.setBindGroup(0, pass.bindGroup);
				render.draw(3);
				render.end();
				continue;
			}
			const compute = encoder.beginComputePass();
			compute.setPipeline(pass.pipeline);
			compute.setBindGroup(0, pass.bindGroup);
			compute.dispatchWorkgroups(Math.ceil(pass.width / 8), Math.ceil(pass.height / 8));
			compute.end();
		}
	}

	private release(): void {
		this.passes = [];
		for (const texture of this.textures) texture.destroy();
		this.textures = [];
		this.output = null;
	}

	destroy(): void { this.release(); }
}
