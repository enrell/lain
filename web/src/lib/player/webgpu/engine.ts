import { planShaderGraph, type ShaderGraphPlan, type ShaderPass } from './graph';
import { VideoFrameScheduler } from './scheduler';
import { DISPLAY_SHADER, effectShader, INGEST_SHADER } from './shaders';
import { TexturePool } from './texture-pool';
import { GPU_COPY_DST, GPU_FRAGMENT_STAGE, GPU_UNIFORM } from './constants';
import type { Anime4KComputePack, Anime4KMode } from './packs/anime4k-compute';
import { DecodedFrameProbe } from '../frame-probe';

const UNIFORM_BYTES = 64;
const MAX_INPUTS = 4;

interface CompiledPass {
	pass: ShaderPass;
	pipeline: GPURenderPipeline;
	uniforms: GPUBuffer;
}

export interface VideoRendererOptions {
	video: HTMLVideoElement;
	canvas: HTMLCanvasElement;
	passes?: readonly ShaderPass[];
	computeMode?: Anime4KMode;
	onActiveChange?: (active: boolean) => void;
	onFailure?: (reason: string) => void;
}

export type VideoRendererStart =
	| { renderer: VideoRenderer; reason?: never }
	| { renderer: null; reason: string };

function errorText(error: unknown): string {
	return error instanceof Error ? error.message : String(error);
}

async function checkedModule(device: GPUDevice, label: string, code: string): Promise<GPUShaderModule> {
	const module = device.createShaderModule({ label, code });
	const info = await module.getCompilationInfo();
	const errors = info.messages.filter((message) => message.type === 'error');
	if (errors.length > 0) {
		throw new Error(`${label}: ${errors.map((message) => message.message).join('; ')}`);
	}
	return module;
}

function colorAttachment(view: GPUTextureView): GPURenderPassColorAttachment {
	return {
		view,
		clearValue: { r: 0, g: 0, b: 0, a: 1 },
		loadOp: 'clear',
		storeOp: 'store'
	};
}

/**
 * Optional presentation layer over the browser media engine. It never owns
 * demux, decode, buffering, seeking, A/V sync or audio.
 */
export class VideoRenderer {
	private readonly pool: TexturePool;
	private readonly scheduler: VideoFrameScheduler;
	private readonly sampler: GPUSampler;
	private plan: ShaderGraphPlan | null = null;
	private frameIndex = 0;
	private stopped = false;
	private active = false;
	private readonly frameProbe = new DecodedFrameProbe();

	private constructor(
		private readonly options: VideoRendererOptions,
		private readonly device: GPUDevice,
		private readonly context: GPUCanvasContext,
		private readonly canvasFormat: GPUTextureFormat,
		private readonly ingestPipeline: GPURenderPipeline,
		private readonly ingestLayout: GPUBindGroupLayout,
		private readonly displayPipeline: GPURenderPipeline,
		private readonly displayLayout: GPUBindGroupLayout,
		private readonly effectLayout: GPUBindGroupLayout,
		private readonly compiled: readonly CompiledPass[],
		private readonly computePack: Anime4KComputePack | null
	) {
		this.pool = new TexturePool(device);
		this.sampler = device.createSampler({
			label: 'shader graph linear sampler',
			magFilter: 'linear',
			minFilter: 'linear',
			mipmapFilter: 'nearest',
			addressModeU: 'clamp-to-edge',
			addressModeV: 'clamp-to-edge'
		});
		this.scheduler = new VideoFrameScheduler(options.video, (_now, metadata) => {
			try {
				this.render(metadata.mediaTime);
			} catch (error) {
				this.fail(errorText(error));
			}
		});
	}

	static async create(options: VideoRendererOptions): Promise<VideoRendererStart> {
		if (!('gpu' in navigator) || !navigator.gpu) {
			return { renderer: null, reason: 'WebGPU is unavailable' };
		}
		if (typeof options.video.requestVideoFrameCallback !== 'function') {
			return { renderer: null, reason: 'decoded-frame callbacks are unavailable' };
		}
		try {
			const adapter =
				(await navigator.gpu.requestAdapter({ powerPreference: 'high-performance' })) ??
				(await navigator.gpu.requestAdapter());
			if (!adapter) return { renderer: null, reason: 'no WebGPU adapter is available' };
			if (options.computeMode && !adapter.features.has('float32-filterable')) {
				return { renderer: null, reason: 'this adapter does not support filterable 32-bit Anime4K textures' };
			}
			const device = await adapter.requestDevice({
				label: 'Lain video shader engine',
				requiredFeatures: options.computeMode ? ['float32-filterable'] : []
			});
			const context = (
				options.canvas as HTMLCanvasElement & {
					getContext(contextId: 'webgpu'): GPUCanvasContext | null;
				}
			).getContext('webgpu');
			if (!context) return { renderer: null, reason: 'WebGPU canvas is unavailable' };
			const canvasFormat = navigator.gpu.getPreferredCanvasFormat();
			context.configure({ device, format: canvasFormat, alphaMode: 'opaque', colorSpace: 'srgb' });

			const ingestLayout = device.createBindGroupLayout({
				label: 'video ingest bindings',
				entries: [
					{ binding: 0, visibility: GPU_FRAGMENT_STAGE, sampler: { type: 'filtering' } },
					{ binding: 1, visibility: GPU_FRAGMENT_STAGE, externalTexture: {} }
				]
			});
			const displayLayout = device.createBindGroupLayout({
				label: 'video display bindings',
				entries: [
					{ binding: 0, visibility: GPU_FRAGMENT_STAGE, sampler: { type: 'filtering' } },
					{ binding: 1, visibility: GPU_FRAGMENT_STAGE, texture: { sampleType: 'float' } }
				]
			});
			const effectLayout = device.createBindGroupLayout({
				label: 'shader pass ABI bindings',
				entries: [
					{ binding: 0, visibility: GPU_FRAGMENT_STAGE, sampler: { type: 'filtering' } },
					...Array.from({ length: MAX_INPUTS }, (_, index): GPUBindGroupLayoutEntry => ({
						binding: index + 1,
						visibility: GPU_FRAGMENT_STAGE,
						texture: { sampleType: 'float' }
					})),
					{ binding: 5, visibility: GPU_FRAGMENT_STAGE, buffer: { type: 'uniform' } }
				]
			});

			const [ingestModule, displayModule] = await Promise.all([
				checkedModule(device, 'video ingest shader', INGEST_SHADER),
				checkedModule(device, 'video display shader', DISPLAY_SHADER)
			]);
			const [ingestPipeline, displayPipeline] = await Promise.all([
				device.createRenderPipelineAsync({
					label: 'video ingest pipeline',
					layout: device.createPipelineLayout({ bindGroupLayouts: [ingestLayout] }),
					vertex: { module: ingestModule, entryPoint: 'vertex_main' },
					fragment: { module: ingestModule, entryPoint: 'fragment_main', targets: [{ format: 'rgba16float' }] },
					primitive: { topology: 'triangle-list' }
				}),
				device.createRenderPipelineAsync({
					label: 'video display pipeline',
					layout: device.createPipelineLayout({ bindGroupLayouts: [displayLayout] }),
					vertex: { module: displayModule, entryPoint: 'vertex_main' },
					fragment: { module: displayModule, entryPoint: 'fragment_main', targets: [{ format: canvasFormat }] },
					primitive: { topology: 'triangle-list' }
				})
			]);

			const passes = options.computeMode ? [] : (options.passes ?? []);
			const compiled = await Promise.all(
				passes.map(async (pass): Promise<CompiledPass> => {
					const module = await checkedModule(device, `shader pass ${pass.id}`, effectShader(pass));
					const pipeline = await device.createRenderPipelineAsync({
						label: `shader pass ${pass.id}`,
						layout: device.createPipelineLayout({ bindGroupLayouts: [effectLayout] }),
						vertex: { module, entryPoint: 'vertex_main' },
						fragment: {
							module,
							entryPoint: 'fragment_main',
							targets: [{ format: pass.outputFormat ?? 'rgba16float' }]
						},
						primitive: { topology: 'triangle-list' }
					});
					return {
						pass,
						pipeline,
						uniforms: device.createBuffer({
							label: `shader pass ${pass.id} uniforms`,
							size: UNIFORM_BYTES,
							usage: GPU_UNIFORM | GPU_COPY_DST
						})
					};
				})
			);
			const computePack = options.computeMode
				? await (await import('./packs/anime4k-compute')).Anime4KComputePack.create(device, options.computeMode)
				: null;

			const renderer = new VideoRenderer(
				options,
				device,
				context,
				canvasFormat,
				ingestPipeline,
				ingestLayout,
				displayPipeline,
				displayLayout,
				effectLayout,
				compiled,
				computePack
			);
			device.lost.then((info) => renderer.fail(`WebGPU device lost: ${info.message || info.reason}`));
			renderer.start();
			return { renderer };
		} catch (error) {
			return { renderer: null, reason: errorText(error) };
		}
	}

	private start(): void {
		this.scheduler.start();
	}

	private ensurePlan(): ShaderGraphPlan | null {
		const { video, canvas, passes = [] } = this.options;
		const width = video.videoWidth;
		const height = video.videoHeight;
		if (width < 1 || height < 1 || video.readyState < HTMLMediaElement.HAVE_CURRENT_DATA) return null;
		if (!this.plan || this.plan.source.width !== width || this.plan.source.height !== height) {
			this.plan = planShaderGraph(this.computePack ? [] : passes, { width, height }, this.device.limits.maxTextureDimension2D);
			this.pool.sync(this.plan);
			if (!this.computePack) {
				if (canvas.width !== this.plan.output.width) canvas.width = this.plan.output.width;
				if (canvas.height !== this.plan.output.height) canvas.height = this.plan.output.height;
			}
		}
		return this.plan;
	}

	private render(mediaTime: number): void {
		if (this.stopped) return;
		const plan = this.ensurePlan();
		if (!plan) return;
		if (!this.frameProbe.check(this.options.video)) {
			if (this.active) {
				this.active = false;
				this.options.onActiveChange?.(false);
			}
			return;
		}
		const encoder = this.device.createCommandEncoder({ label: `video frame ${this.frameIndex}` });
		const external = this.device.importExternalTexture({ source: this.options.video, colorSpace: 'srgb' });
		const ingestBindings = this.device.createBindGroup({
			label: 'video ingest frame bindings',
			layout: this.ingestLayout,
			entries: [
				{ binding: 0, resource: this.sampler },
				{ binding: 1, resource: external }
			]
		});
		const ingest = encoder.beginRenderPass({
			label: 'video ingest pass',
			colorAttachments: [colorAttachment(this.pool.get(plan.sourceSlot).createView())]
		});
		ingest.setPipeline(this.ingestPipeline);
		ingest.setBindGroup(0, ingestBindings);
		ingest.draw(3);
		ingest.end();

		for (let index = 0; index < plan.passes.length; index++) {
			const node = plan.passes[index];
			const compiled = this.compiled[index];
			const primary = node.inputs[0];
			const uniformData = new ArrayBuffer(UNIFORM_BYTES);
			const floats = new Float32Array(uniformData);
			const uints = new Uint32Array(uniformData);
			floats[0] = primary.width;
			floats[1] = primary.height;
			floats[2] = node.width;
			floats[3] = node.height;
			floats[4] = mediaTime;
			uints[5] = this.frameIndex;
			for (let param = 0; param < Math.min(8, compiled.pass.params?.length ?? 0); param++) {
				floats[8 + param] = compiled.pass.params![param];
			}
			this.device.queue.writeBuffer(compiled.uniforms, 0, uniformData);

			const textures = Array.from({ length: MAX_INPUTS }, (_, input) => {
				const selected = node.inputs[input] ?? primary;
				return this.pool.get(selected.slot).createView();
			});
			const bindings = this.device.createBindGroup({
				label: `shader pass ${node.id} frame bindings`,
				layout: this.effectLayout,
				entries: [
					{ binding: 0, resource: this.sampler },
					...textures.map((view, input) => ({ binding: input + 1, resource: view })),
					{ binding: 5, resource: { buffer: compiled.uniforms } }
				]
			});
			const effect = encoder.beginRenderPass({
				label: `shader pass ${node.id}`,
				colorAttachments: [colorAttachment(this.pool.get(node.outputSlot).createView())]
			});
			effect.setPipeline(compiled.pipeline);
			effect.setBindGroup(0, bindings);
			effect.draw(3);
			effect.end();
		}

		let outputTexture = this.pool.get(plan.output.slot);
		if (this.computePack) {
			const clientWidth = Math.max(1, this.options.canvas.clientWidth);
			const clientHeight = Math.max(1, this.options.canvas.clientHeight);
			const fit = Math.min(clientWidth / plan.source.width, clientHeight / plan.source.height);
			outputTexture = this.computePack.configure(
				this.pool.get(plan.sourceSlot),
				Math.round(plan.source.width * fit),
				Math.round(plan.source.height * fit)
			);
			this.computePack.encode(encoder);
			if (this.options.canvas.width !== outputTexture.width) this.options.canvas.width = outputTexture.width;
			if (this.options.canvas.height !== outputTexture.height) this.options.canvas.height = outputTexture.height;
		}
		const displayBindings = this.device.createBindGroup({
			label: 'video display frame bindings',
			layout: this.displayLayout,
			entries: [
				{ binding: 0, resource: this.sampler },
				{ binding: 1, resource: outputTexture.createView() }
			]
		});
		const display = encoder.beginRenderPass({
			label: 'video display pass',
			colorAttachments: [colorAttachment(this.context.getCurrentTexture().createView())]
		});
		display.setPipeline(this.displayPipeline);
		display.setBindGroup(0, displayBindings);
		display.draw(3);
		display.end();

		this.device.queue.submit([encoder.finish()]);
		if (!this.active) {
			this.active = true;
			this.options.onActiveChange?.(true);
		}
		this.frameIndex = (this.frameIndex + 1) >>> 0;
	}

	private fail(reason: string): void {
		if (this.stopped) return;
		this.stop();
		this.options.onFailure?.(reason);
	}

	stop(): void {
		if (this.stopped) return;
		this.stopped = true;
		this.scheduler.stop();
		this.pool.destroy();
		this.computePack?.destroy();
		for (const pass of this.compiled) pass.uniforms.destroy();
		this.context.unconfigure();
		if (this.active) this.options.onActiveChange?.(false);
		this.active = false;
	}
}
