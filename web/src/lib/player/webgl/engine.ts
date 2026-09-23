import { VideoFrameScheduler } from '../webgpu/scheduler';
import { anime4kHooks, type HookPass } from './packs/anime4k';
import type { EffectPreset } from '../effects-policy';
import { DecodedFrameProbe } from '../frame-probe';

interface ImageTexture { texture: WebGLTexture; width: number; height: number }
interface PreparedPass { hook: HookPass; program: WebGLProgram; inputs: string[] }
interface RenderPass { prepared: PreparedPass; bindings: ImageTexture[]; output: ImageTexture }

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

function fragmentSource(hook: HookPass, inputs: string[]): string {
	const helpers = inputs.map((name) => `
uniform sampler2D u_${name};
uniform vec2 size_${name};
#define ${name}_pos v_uv
#define ${name}_pt (1.0 / size_${name})
#define ${name}_size size_${name}
vec4 ${name}_tex(vec2 uv) { return texture(u_${name}, uv); }
vec4 ${name}_texOff(vec2 offset) { return texture(u_${name}, v_uv + offset / size_${name}); }
`).join('');
	return `#version 300 es
precision highp float;
precision highp sampler2D;
in vec2 v_uv;
out vec4 out_color;
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

/** Runs the same local mpv GLSL hooks when Linux Brave has no WebGPU adapter. */
export class WebGLAnime4KRenderer {
	private readonly scheduler: VideoFrameScheduler;
	private readonly frameBuffer: WebGLFramebuffer;
	private readonly vertexArray: WebGLVertexArrayObject;
	private readonly vertex: WebGLShader;
	private readonly display: WebGLProgram;
	private readonly prepared: PreparedPass[] = [];
	private readonly textures: ImageTexture[] = [];
	private source: ImageTexture | null = null;
	private renderPasses: RenderPass[] = [];
	private output: ImageTexture | null = null;
	private active = false;
	private stopped = false;
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
		this.frameBuffer = gl.createFramebuffer() ?? fail('WebGL framebuffer allocation failed');
		this.vertexArray = gl.createVertexArray() ?? fail('WebGL vertex array allocation failed');
		for (const hook of anime4kHooks(mode)) {
			const inputs = names(hook);
			if (inputs.length > gl.getParameter(gl.MAX_TEXTURE_IMAGE_UNITS)) fail(`${hook.description}: too many textures`);
			this.prepared.push({ hook, inputs, program: program(gl, this.vertex, fragmentSource(hook, inputs)) });
		}
		this.scheduler = new VideoFrameScheduler(video, () => {
			try { this.render(); }
			catch (error) { this.fail(error instanceof Error ? error.message : String(error)); }
		});
	}

	static create(options: {
		video: HTMLVideoElement; canvas: HTMLCanvasElement; mode: EffectPreset;
		onActiveChange?: (active: boolean) => void; onFailure?: (reason: string) => void;
	}): { renderer: WebGLAnime4KRenderer | null; reason?: string } {
		if (typeof options.video.requestVideoFrameCallback !== 'function') {
			return { renderer: null, reason: 'decoded-frame callbacks are unavailable' };
		}
		const gl = options.canvas.getContext('webgl2', { alpha: false, antialias: false, preserveDrawingBuffer: false });
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
			this.renderPasses.push({ prepared, bindings: inputTextures, output });
			bindings.set(hook.save, output);
			dimensions.set(hook.save, { width: outputWidth, height: outputHeight });
		}
		this.output = bindings.get('MAIN')!;
		this.canvas.width = this.output.width;
		this.canvas.height = this.output.height;
	}

	private render(): void {
		if (this.stopped || this.video.readyState < HTMLMediaElement.HAVE_CURRENT_DATA) return;
		const gl = this.gl;
		const width = this.video.videoWidth;
		const height = this.video.videoHeight;
		if (!width || !height) return;
		const readable = this.frameProbe.check(this.video);
		if (!readable) {
			if (this.active) { this.active = false; this.onActiveChange?.(false); }
			return;
		}
		const fit = Math.min(Math.max(1, this.canvas.clientWidth) / width, Math.max(1, this.canvas.clientHeight) / height);
		const targetWidth = Math.max(1, Math.round(width * fit));
		const targetHeight = Math.max(1, Math.round(height * fit));
		const dimensions = `${width}:${height}:${targetWidth}:${targetHeight}`;
		if (dimensions !== this.lastDimensions) {
			this.prepare(width, height, targetWidth, targetHeight);
			this.lastDimensions = dimensions;
		}
		gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL, true);
		gl.bindTexture(gl.TEXTURE_2D, this.source!.texture);
		gl.texSubImage2D(gl.TEXTURE_2D, 0, 0, 0, gl.RGBA, gl.UNSIGNED_BYTE, this.video);
		gl.bindVertexArray(this.vertexArray);
		gl.bindFramebuffer(gl.FRAMEBUFFER, this.frameBuffer);
		for (const pass of this.renderPasses) {
			gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, pass.output.texture, 0);
			if (gl.checkFramebufferStatus(gl.FRAMEBUFFER) !== gl.FRAMEBUFFER_COMPLETE) fail(`${pass.prepared.hook.description}: incomplete render target`);
			gl.viewport(0, 0, pass.output.width, pass.output.height);
			gl.useProgram(pass.prepared.program);
			pass.bindings.forEach((binding, index) => {
				const name = pass.prepared.inputs[index];
				gl.activeTexture(gl.TEXTURE0 + index);
				gl.bindTexture(gl.TEXTURE_2D, binding.texture);
				gl.uniform1i(gl.getUniformLocation(pass.prepared.program, `u_${name}`), index);
				gl.uniform2f(gl.getUniformLocation(pass.prepared.program, `size_${name}`), binding.width, binding.height);
			});
			gl.drawArrays(gl.TRIANGLES, 0, 3);
		}
		gl.bindFramebuffer(gl.FRAMEBUFFER, null);
		gl.viewport(0, 0, this.canvas.width, this.canvas.height);
		gl.useProgram(this.display);
		gl.activeTexture(gl.TEXTURE0);
		gl.bindTexture(gl.TEXTURE_2D, this.output!.texture);
		gl.uniform1i(gl.getUniformLocation(this.display, 'u_image'), 0);
		gl.drawArrays(gl.TRIANGLES, 0, 3);
		if (gl.getError() !== gl.NO_ERROR) fail('WebGL Anime4K frame failed');
		if (!this.active) { this.active = true; this.onActiveChange?.(true); }
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
