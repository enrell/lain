import { VideoFrameScheduler } from '../webgpu/scheduler';
import { anime4kHooks, type HookPass } from './packs/anime4k';
import type { EffectPreset } from '../effects-policy';
import { DecodedFrameProbe } from '../frame-probe';

interface ImageTexture { texture: WebGLTexture; width: number; height: number }
interface PassUniforms { sampler: WebGLUniformLocation | null; size: WebGLUniformLocation | null }
interface PreparedPass { hook: HookPass; program: WebGLProgram; inputs: string[]; uniforms: PassUniforms[] }
// The last pass renders straight to the canvas, so its output is null there.
interface RenderPass { prepared: PreparedPass; bindings: ImageTexture[]; output: ImageTexture | null; width: number; height: number }

const VERTEX = `#version 300 es
precision highp float;
out vec2 v_uv;
void main() {
  vec2 p = vec2((gl_VertexID << 1) & 2, gl_VertexID & 2);
  v_uv = p;
  gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0);
}`;
const DISPLAY = `#version 300 es
precision highp float;
uniform sampler2D u_image;
in vec2 v_uv;
out vec4 out_color;
void main() { out_color = texture(u_image, v_uv); }`;

function fail(message: string): never { throw new Error(message); }

function shader(gl: WebGL2RenderingContext, type: number, source: string): WebGLShader {
	const result = gl.createShader(type) ?? fail('WebGL shader allocation failed');
	gl.shaderSource(result, source);
	gl.compileShader(result);
	if (!gl.getShaderParameter(result, gl.COMPILE_STATUS)) {
		const message = gl.getShaderInfoLog(result) ?? 'unknown GLSL error';
		gl.deleteShader(result);
		fail(message);
	}
	return result;
}

function program(gl: WebGL2RenderingContext, vertex: WebGLShader, fragment: string): WebGLProgram {
	const compiled = shader(gl, gl.FRAGMENT_SHADER, fragment);
	const result = gl.createProgram() ?? fail('WebGL program allocation failed');
	gl.attachShader(result, vertex);
	gl.attachShader(result, compiled);
	gl.linkProgram(result);
	gl.deleteShader(compiled);
	if (!gl.getProgramParameter(result, gl.LINK_STATUS)) {
		const message = gl.getProgramInfoLog(result) ?? 'unknown GLSL link error';
		gl.deleteProgram(result);
		fail(message);
	}
	return result;
}

// Convolution layers are pure multiply-accumulate over fp16 render targets,
// so they run at mediump: GPUs with packed half-precision math (RDNA2 and
// newer, most mobile parts) execute them at up to twice the rate, and the
// result differs from highp by at most 1/255 (verified on a 1080p frame).
// Sampling coordinates, sizes and non-convolution hooks stay highp because
// half-float uv cannot address a full-HD row.
function fragmentSource(hook: HookPass, inputs: string[]): string {
	const p = /\bmat4\(/.test(hook.source) ? 'mediump' : 'highp';
	const helpers = inputs.map((name) => `
uniform highp sampler2D u_${name};
uniform highp vec2 size_${name};
#define ${name}_pos v_uv
#define ${name}_pt (1.0 / size_${name})
#define ${name}_size size_${name}
${p} vec4 ${name}_tex(highp vec2 uv) { return texture(u_${name}, uv); }
${p} vec4 ${name}_texOff(highp vec2 offset) { return texture(u_${name}, v_uv + offset / size_${name}); }
`).join('');
	return `#version 300 es
precision ${p} float;
precision highp sampler2D;
in highp vec2 v_uv;
out ${p} vec4 out_color;
${helpers}
${hook.source}
void main() { out_color = hook(); }
`;
}

function names(hook: HookPass): string[] {
	const referenced = [...hook.source.matchAll(/\b([A-Za-z][A-Za-z0-9_]*)_(?:texOff|tex|pos|pt|size)\b/g)]
		.map((match) => match[1]).filter((name) => name !== 'L');
	return [...new Set([...hook.binds, ...referenced])];
}

function expression(source: string, dimensions: Map<string, { width: number; height: number }>): number {
	const stack: number[] = [];
	for (const token of source.split(/\s+/)) {
		if (/^-?\d+(?:\.\d+)?$/.test(token)) { stack.push(Number(token)); continue; }
		const field = /^([A-Za-z][A-Za-z0-9_]*)\.([wh])$/.exec(token);
		if (field) {
			const size = dimensions.get(field[1]);
			if (!size) fail(`Anime4K dimension ${token} is unavailable`);
			stack.push(field[2] === 'w' ? size.width : size.height);
			continue;
		}
		const right = stack.pop();
		const left = stack.pop();
		if (left === undefined || right === undefined) fail(`Invalid Anime4K expression: ${source}`);
		switch (token) {
			case '+': stack.push(left + right); break;
			case '-': stack.push(left - right); break;
			case '*': stack.push(left * right); break;
			case '/': stack.push(left / right); break;
			case '>': stack.push(Number(left > right)); break;
			case '<': stack.push(Number(left < right)); break;
			case '>=': stack.push(Number(left >= right)); break;
			case '<=': stack.push(Number(left <= right)); break;
			default: fail(`Unknown Anime4K expression operator ${token}`);
		}
	}
	if (stack.length !== 1 || !Number.isFinite(stack[0])) fail(`Invalid Anime4K expression: ${source}`);
	return stack[0];
}

function image(gl: WebGL2RenderingContext, width: number, height: number, format: number): ImageTexture {
	const texture = gl.createTexture() ?? fail('WebGL texture allocation failed');
	gl.bindTexture(gl.TEXTURE_2D, texture);
	gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
	gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
	gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
	gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
	gl.texStorage2D(gl.TEXTURE_2D, 1, format, width, height);
	return { texture, width, height };
}

/** Runs the bundled mpv GLSL hooks in browsers whose profile prefers WebGL2. */
export class WebGLAnime4KRenderer {
	private readonly scheduler: VideoFrameScheduler;
	private readonly frameBuffer: WebGLFramebuffer;
	private readonly vertexArray: WebGLVertexArrayObject;
	private readonly vertex: WebGLShader;
	private readonly display: WebGLProgram;
	private readonly displayUniform: WebGLUniformLocation | null;
	private readonly prepared: PreparedPass[] = [];
	private readonly textures: ImageTexture[] = [];
	private source: ImageTexture | null = null;
	private renderPasses: RenderPass[] = [];
	private active = false;
	private stopped = false;
	private renderInFlight = false;
	private frames = 0;
	private lastDimensions = '';
	private readonly frameProbe = new DecodedFrameProbe();

	private constructor(
		private readonly video: HTMLVideoElement,
		private readonly canvas: HTMLCanvasElement,
		private readonly gl: WebGL2RenderingContext,
		private readonly onActiveChange?: (active: boolean) => void,
		private readonly onFailure?: (reason: string) => void,
		mode: EffectPreset = 'anime4k-lite'
	) {
		this.vertex = shader(gl, gl.VERTEX_SHADER, VERTEX);
		this.display = program(gl, this.vertex, DISPLAY);
		this.displayUniform = gl.getUniformLocation(this.display, 'u_image');
		this.frameBuffer = gl.createFramebuffer() ?? fail('WebGL framebuffer allocation failed');
		this.vertexArray = gl.createVertexArray() ?? fail('WebGL vertex array allocation failed');
		gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL, true);
		for (const hook of anime4kHooks(mode)) {
			const inputs = names(hook);
			if (inputs.length > gl.getParameter(gl.MAX_TEXTURE_IMAGE_UNITS)) fail(`${hook.description}: too many textures`);
			const passProgram = program(gl, this.vertex, fragmentSource(hook, inputs));
			const uniforms = inputs.map((name) => ({
				sampler: gl.getUniformLocation(passProgram, `u_${name}`),
				size: gl.getUniformLocation(passProgram, `size_${name}`)
			}));
			this.prepared.push({ hook, inputs, program: passProgram, uniforms });
		}
		this.scheduler = new VideoFrameScheduler(video, () => this.render());
	}

	static create(options: {
		video: HTMLVideoElement; canvas: HTMLCanvasElement; mode: EffectPreset;
		onActiveChange?: (active: boolean) => void; onFailure?: (reason: string) => void;
	}): { renderer: WebGLAnime4KRenderer | null; reason?: string } {
		if (typeof options.video.requestVideoFrameCallback !== 'function') {
			return { renderer: null, reason: 'decoded-frame callbacks are unavailable' };
		}
		// No depth/stencil buffers: every draw is a fullscreen triangle, so the
		// default attachment block would only tax framebuffer bandwidth on
		// weak GPUs. desynchronized skips compositor-side copy where the
		// driver honors the hint.
		const gl = options.canvas.getContext('webgl2', {
			alpha: false, antialias: false, depth: false, stencil: false,
			preserveDrawingBuffer: false, desynchronized: true,
			powerPreference: 'high-performance'
		});
		if (!gl) return { renderer: null, reason: 'WebGL2 is unavailable' };
		if (!gl.getExtension('EXT_color_buffer_float')) return { renderer: null, reason: 'floating-point WebGL render targets are unavailable' };
		try {
			const renderer = new WebGLAnime4KRenderer(options.video, options.canvas, gl, options.onActiveChange, options.onFailure, options.mode);
			renderer.scheduler.start();
			return { renderer };
		} catch (error) {
			return { renderer: null, reason: error instanceof Error ? error.message : String(error) };
		}
	}

	private prepare(width: number, height: number, targetWidth: number, targetHeight: number): void {
		const gl = this.gl;
		for (const previous of this.textures) gl.deleteTexture(previous.texture);
		this.textures.length = 0;
		this.source = image(gl, width, height, gl.RGBA8);
		this.textures.push(this.source);
		const dimensions = new Map<string, { width: number; height: number }>([
			['NATIVE', { width, height }], ['MAIN', { width, height }],
			['OUTPUT', { width: targetWidth, height: targetHeight }]
		]);
		const bindings = new Map<string, ImageTexture>([['NATIVE', this.source], ['MAIN', this.source]]);
		this.renderPasses = [];
		gl.bindFramebuffer(gl.FRAMEBUFFER, this.frameBuffer);
		for (const prepared of this.prepared) {
			const hook = prepared.hook;
			const main = bindings.get('MAIN')!;
			bindings.set('HOOKED', main);
			dimensions.set('HOOKED', { width: main.width, height: main.height });
			if (hook.when && expression(hook.when, dimensions) === 0) continue;
			const outputWidth = Math.max(1, Math.round(hook.width ? expression(hook.width, dimensions) : main.width));
			const outputHeight = Math.max(1, Math.round(hook.height ? expression(hook.height, dimensions) : main.height));
			if (outputWidth > gl.getParameter(gl.MAX_TEXTURE_SIZE) || outputHeight > gl.getParameter(gl.MAX_TEXTURE_SIZE)) {
				fail(`Anime4K texture ${outputWidth}x${outputHeight} exceeds GPU limits`);
			}
			const inputTextures = prepared.inputs.map((name) => bindings.get(name) ?? fail(`${hook.description}: missing ${name}`));
			const output = image(gl, outputWidth, outputHeight, gl.RGBA16F);
			this.textures.push(output);
			gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, output.texture, 0);
			// FBO completeness is a property of the allocated texture, so it is
			// proven once here instead of before every presented frame.
			if (gl.checkFramebufferStatus(gl.FRAMEBUFFER) !== gl.FRAMEBUFFER_COMPLETE) {
				fail(`${hook.description}: incomplete render target`);
			}
			this.renderPasses.push({ prepared, bindings: inputTextures, output, width: outputWidth, height: outputHeight });
			bindings.set(hook.save, output);
			dimensions.set(hook.save, { width: outputWidth, height: outputHeight });
		}
		gl.bindFramebuffer(gl.FRAMEBUFFER, null);
		// The final pass draws straight to the canvas, so it needs no output
		// texture and no display blit afterwards — significant on GPUs where
		// the upscale target is a 4K float texture. `when` skips make the
		// chain's last *rendered* pass the candidate, not the last prepared.
		const finalPass = this.renderPasses.at(-1);
		if (finalPass?.output) {
			gl.deleteTexture(finalPass.output.texture);
			this.textures.splice(this.textures.indexOf(finalPass.output), 1);
			finalPass.output = null;
		}
		this.canvas.width = finalPass ? finalPass.width : width;
		this.canvas.height = finalPass ? finalPass.height : height;
	}

	private render(): void {
		// A presented frame that arrives while the previous render is still on
		// the GPU is dropped rather than queued, so the engine can never pile
		// up work behind playback.
		if (this.renderInFlight) return;
		this.renderInFlight = true;
		void this.renderFrame()
			.catch((error) => this.fail(error instanceof Error ? error.message : String(error)))
			.finally(() => { this.renderInFlight = false; });
	}

	private async renderFrame(): Promise<void> {
		if (this.stopped || this.video.readyState < HTMLMediaElement.HAVE_CURRENT_DATA) return;
		const width = this.video.videoWidth;
		const height = this.video.videoHeight;
		if (!width || !height) return;
		// texSubImage2D(video) costs ~33ms on GPUs whose decoded frames need a
		// GPU→CPU→GPU roundtrip; createImageBitmap keeps the frame GPU-side and
		// uploads in ~0.1ms on the same hardware. Bitmap uploads ignore
		// UNPACK_FLIP_Y_WEBGL, so the vertical flip the pipeline expects moves
		// into the bitmap's own imageOrientation instead.
		const bitmap = await createImageBitmap(this.video, { imageOrientation: 'flipY' });
		try {
			if (this.stopped) return;
			const readable = this.frameProbe.check(bitmap);
			if (!readable) {
				if (this.active) { this.active = false; this.onActiveChange?.(false); }
				return;
			}
			const gl = this.gl;
			const fit = Math.min(Math.max(1, this.canvas.clientWidth) / width, Math.max(1, this.canvas.clientHeight) / height);
			const targetWidth = Math.max(1, Math.round(width * fit));
			const targetHeight = Math.max(1, Math.round(height * fit));
			const dimensions = `${width}:${height}:${targetWidth}:${targetHeight}`;
			if (dimensions !== this.lastDimensions) {
				this.prepare(width, height, targetWidth, targetHeight);
				this.lastDimensions = dimensions;
			}
			gl.bindTexture(gl.TEXTURE_2D, this.source!.texture);
			gl.texSubImage2D(gl.TEXTURE_2D, 0, 0, 0, gl.RGBA, gl.UNSIGNED_BYTE, bitmap);
			gl.bindVertexArray(this.vertexArray);
			for (const pass of this.renderPasses) {
				if (pass.output) {
					gl.bindFramebuffer(gl.FRAMEBUFFER, this.frameBuffer);
					gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, pass.output.texture, 0);
					gl.viewport(0, 0, pass.width, pass.height);
				} else {
					gl.bindFramebuffer(gl.FRAMEBUFFER, null);
					gl.viewport(0, 0, this.canvas.width, this.canvas.height);
				}
				gl.useProgram(pass.prepared.program);
				pass.bindings.forEach((binding, index) => {
					const uniforms = pass.prepared.uniforms[index];
					gl.activeTexture(gl.TEXTURE0 + index);
					gl.bindTexture(gl.TEXTURE_2D, binding.texture);
					gl.uniform1i(uniforms.sampler, index);
					gl.uniform2f(uniforms.size, binding.width, binding.height);
				});
				gl.drawArrays(gl.TRIANGLES, 0, 3);
			}
			if (this.renderPasses.length === 0) {
				gl.bindFramebuffer(gl.FRAMEBUFFER, null);
				gl.viewport(0, 0, this.canvas.width, this.canvas.height);
				gl.useProgram(this.display);
				gl.activeTexture(gl.TEXTURE0);
				gl.bindTexture(gl.TEXTURE_2D, this.source!.texture);
				gl.uniform1i(this.displayUniform, 0);
				gl.drawArrays(gl.TRIANGLES, 0, 3);
			}
			// getError syncs the GL pipeline, so it proves the activation
			// frames and then only samples occasionally instead of stalling
			// every presented frame.
			this.frames = (this.frames + 1) % 240;
			if ((!this.active || this.frames === 0) && gl.getError() !== gl.NO_ERROR) {
				fail('WebGL Anime4K frame failed');
			}
			if (!this.active) { this.active = true; this.onActiveChange?.(true); }
		} finally {
			bitmap.close();
		}
	}

	private fail(reason: string): void {
		if (this.stopped) return;
		this.stop();
		this.onFailure?.(reason);
	}

	stop(): void {
		if (this.stopped) return;
		this.stopped = true;
		this.scheduler.stop();
		if (this.active) this.onActiveChange?.(false);
		for (const texture of this.textures) this.gl.deleteTexture(texture.texture);
		for (const pass of this.prepared) this.gl.deleteProgram(pass.program);
		this.gl.deleteProgram(this.display);
		this.gl.deleteShader(this.vertex);
		this.gl.deleteFramebuffer(this.frameBuffer);
		this.gl.deleteVertexArray(this.vertexArray);
	}
}
