import type { ShaderGraphPlan } from './graph';
import { GPU_RENDER_ATTACHMENT, GPU_TEXTURE_BINDING } from './constants';

interface PooledTexture {
	spec: string;
	texture: GPUTexture;
}

function specKey(width: number, height: number, format: GPUTextureFormat): string {
	return `${width}x${height}:${format}`;
}

/** Owns the graph's stable slot set and only reallocates after a size change. */
export class TexturePool {
	private readonly textures = new Map<number, PooledTexture>();

	constructor(private readonly device: GPUDevice) {}

	sync(plan: ShaderGraphPlan): void {
		const live = new Set(plan.slots.map((slot) => slot.slot));
		for (const [slot, pooled] of this.textures) {
			if (!live.has(slot)) {
				pooled.texture.destroy();
				this.textures.delete(slot);
			}
		}

		for (const slot of plan.slots) {
			const spec = specKey(slot.width, slot.height, slot.format);
			const existing = this.textures.get(slot.slot);
			if (existing?.spec === spec) continue;
			existing?.texture.destroy();
			this.textures.set(slot.slot, {
				spec,
				texture: this.device.createTexture({
					label: `shader graph slot ${slot.slot}`,
					size: { width: slot.width, height: slot.height },
					format: slot.format,
					usage: GPU_RENDER_ATTACHMENT | GPU_TEXTURE_BINDING
				})
			});
		}
	}

	get(slot: number): GPUTexture {
		const texture = this.textures.get(slot)?.texture;
		if (!texture) throw new Error(`shader graph texture slot ${slot} is not allocated`);
		return texture;
	}

	destroy(): void {
		for (const pooled of this.textures.values()) pooled.texture.destroy();
		this.textures.clear();
	}
}
