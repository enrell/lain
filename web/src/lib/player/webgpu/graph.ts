/** Reserved name for the RGB texture produced by the video ingest pass. */
export const SOURCE_NODE = '$source';

/**
 * WGSL pass contract. The source supplies `fn effect(uv: vec2f) -> vec4f`.
 * The engine supplies the fixed bindings and helpers documented in
 * docs/SHADER_ENGINE.md.
 */
export interface ShaderPass {
	id: string;
	label: string;
	wgsl: string;
	/** Earlier pass ids. Omitted means the immediately preceding output. */
	inputs?: readonly string[];
	/** Output dimensions relative to the first input. */
	scale?: number;
	outputFormat?: GPUTextureFormat;
	/** Eight scalar values exposed as params.values0/values1. */
	params?: readonly number[];
}

export interface TextureSize {
	width: number;
	height: number;
}

export interface PlannedInput extends TextureSize {
	id: string;
	slot: number;
}

export interface PlannedPass extends TextureSize {
	id: string;
	pass: ShaderPass;
	inputs: PlannedInput[];
	outputSlot: number;
	format: GPUTextureFormat;
	pixelCost: number;
}

export interface PlannedTexture extends TextureSize {
	slot: number;
	format: GPUTextureFormat;
}

export interface ShaderGraphPlan {
	source: TextureSize;
	sourceSlot: number;
	passes: PlannedPass[];
	slots: PlannedTexture[];
	output: PlannedInput & { format: GPUTextureFormat };
	estimatedPixels: number;
}

interface NodeLifetime extends TextureSize {
	id: string;
	format: GPUTextureFormat;
	createdAt: number;
	lastUse: number;
	slot: number;
}

function dimensions(size: TextureSize): TextureSize {
	if (!Number.isInteger(size.width) || !Number.isInteger(size.height) || size.width < 1 || size.height < 1) {
		throw new Error('shader graph source dimensions must be positive integers');
	}
	return size;
}

function passID(id: string): string {
	const clean = id.trim();
	if (!clean || clean === SOURCE_NODE) throw new Error(`invalid shader pass id ${JSON.stringify(id)}`);
	return clean;
}

function textureKey(node: Pick<NodeLifetime, 'width' | 'height' | 'format'>): string {
	return `${node.width}x${node.height}:${node.format}`;
}

/**
 * Validates an ordered DAG, resolves dimensions and assigns the minimum safe
 * set of reusable textures. A texture is not reusable by a pass that still
 * reads it, which naturally yields A/B ping-pong for a linear same-size graph.
 */
export function planShaderGraph(
	passes: readonly ShaderPass[],
	sourceSize: TextureSize,
	maxTextureDimension = Number.POSITIVE_INFINITY
): ShaderGraphPlan {
	const source = dimensions(sourceSize);
	const known = new Map<string, NodeLifetime>();
	const nodes: NodeLifetime[] = [
		{
			id: SOURCE_NODE,
			...source,
			format: 'rgba16float',
			createdAt: -1,
			lastUse: -1,
			slot: -1
		}
	];
	known.set(SOURCE_NODE, nodes[0]);

	const resolvedInputs: string[][] = [];
	for (let index = 0; index < passes.length; index++) {
		const pass = passes[index];
		const id = passID(pass.id);
		if (known.has(id)) throw new Error(`duplicate pass id ${JSON.stringify(id)}`);
		const fallback = index === 0 ? SOURCE_NODE : nodes[nodes.length - 1].id;
		const inputs = [...(pass.inputs?.length ? pass.inputs : [fallback])];
		if (inputs.length > 4) {
			throw new Error(`pass ${JSON.stringify(id)} has ${inputs.length} inputs; the WGSL ABI supports 4`);
		}
		if ((pass.params?.length ?? 0) > 8) {
			throw new Error(`pass ${JSON.stringify(id)} has ${pass.params!.length} parameters; the WGSL ABI supports 8`);
		}
		for (const input of inputs) {
			const node = known.get(input);
			if (!node) throw new Error(`pass ${JSON.stringify(id)} has unknown or forward input ${JSON.stringify(input)}`);
			node.lastUse = Math.max(node.lastUse, index);
		}
		const primary = known.get(inputs[0])!;
		const scale = pass.scale ?? 1;
		if (!Number.isFinite(scale) || scale <= 0) {
			throw new Error(`pass ${JSON.stringify(id)} has invalid scale ${scale}`);
		}
		const width = Math.max(1, Math.round(primary.width * scale));
		const height = Math.max(1, Math.round(primary.height * scale));
		if (width > maxTextureDimension || height > maxTextureDimension) {
			throw new Error(
				`pass ${JSON.stringify(id)} output ${width}x${height} exceeds the device limit ${maxTextureDimension}`
			);
		}
		const node: NodeLifetime = {
			id,
			width,
			height,
			format: pass.outputFormat ?? 'rgba16float',
			createdAt: index,
			lastUse: index,
			slot: -1
		};
		nodes.push(node);
		known.set(id, node);
		resolvedInputs.push(inputs);
	}

	const outputNode = nodes[nodes.length - 1];
	// The display pass consumes the graph output after every effect pass.
	outputNode.lastUse = Math.max(outputNode.lastUse, passes.length);

	const slots: PlannedTexture[] = [];
	const slotRelease: number[] = [];
	const slotKeys: string[] = [];
	for (const node of nodes) {
		const key = textureKey(node);
		let slot = slotKeys.findIndex(
			(candidate, candidateSlot) => candidate === key && slotRelease[candidateSlot] < node.createdAt
		);
		if (slot < 0) {
			slot = slots.length;
			slots.push({ slot, width: node.width, height: node.height, format: node.format });
			slotKeys.push(key);
			slotRelease.push(node.lastUse);
		} else {
			slotRelease[slot] = node.lastUse;
		}
		node.slot = slot;
	}

	const planned = passes.map((pass, index): PlannedPass => {
		const node = known.get(pass.id.trim())!;
		return {
			id: node.id,
			pass,
			width: node.width,
			height: node.height,
			format: node.format,
			outputSlot: node.slot,
			pixelCost: node.width * node.height,
			inputs: resolvedInputs[index].map((id) => {
				const input = known.get(id)!;
				return { id, slot: input.slot, width: input.width, height: input.height };
			})
		};
	});

	return {
		source,
		sourceSlot: known.get(SOURCE_NODE)!.slot,
		passes: planned,
		slots,
		output: {
			id: outputNode.id,
			slot: outputNode.slot,
			width: outputNode.width,
			height: outputNode.height,
			format: outputNode.format
		},
		estimatedPixels: planned.reduce((total, node) => total + node.pixelCost, 0)
	};
}
