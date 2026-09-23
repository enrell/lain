import { prefs } from '$lib/auth/storage';

export const EFFECT_PRESETS = [
	{ id: 'off', label: 'Off' },
	{ id: 'anime4k-a', label: 'Anime4K Mode A (Ctrl+1)' },
	{ id: 'anime4k-aa', label: 'Anime4K Mode A+A (Ctrl+2)' },
	{ id: 'anime4k-lite', label: 'Anime4K Lite (no upscale)' },
	{ id: 'anime4k-dog-x2', label: 'Anime4K DoG ×2' }
] as const;

export type EffectPreset = (typeof EFFECT_PRESETS)[number]['id'];

export interface EffectRule {
	libraryType?: string;
	libraryId?: string;
	streamIndex?: number;
	minHeight?: number;
	maxHeight?: number;
	preset: EffectPreset;
}

export interface EffectsPolicy {
	default: EffectPreset;
	rules: EffectRule[];
}

export interface EffectContext {
	libraryId: string;
	libraryType: string;
	streamIndex: number;
	height: number;
}

export function defaultEffectsPolicy(): EffectsPolicy {
	return { default: 'off', rules: [] };
}

function isPreset(value: unknown): value is EffectPreset {
	return EFFECT_PRESETS.some((preset) => preset.id === value);
}

export function validateEffectsPolicy(value: unknown): EffectsPolicy | null {
	if (!value || typeof value !== 'object') return null;
	const policy = value as Record<string, unknown>;
	if (!isPreset(policy.default) || !Array.isArray(policy.rules) || policy.rules.length > 128) return null;
	const rules: EffectRule[] = [];
	for (const raw of policy.rules) {
		if (!raw || typeof raw !== 'object') return null;
		const rule = raw as Record<string, unknown>;
		if (!isPreset(rule.preset)) return null;
		if (rule.libraryType !== undefined && (typeof rule.libraryType !== 'string' || !rule.libraryType)) return null;
		if (rule.libraryId !== undefined && (typeof rule.libraryId !== 'string' || !rule.libraryId)) return null;
		for (const key of ['streamIndex', 'minHeight', 'maxHeight'] as const) {
			if (rule[key] !== undefined && (!Number.isInteger(rule[key]) || (rule[key] as number) < 0)) return null;
		}
		if (rule.minHeight !== undefined && rule.maxHeight !== undefined && (rule.minHeight as number) > (rule.maxHeight as number)) return null;
		if (rule.libraryType === undefined && rule.libraryId === undefined && rule.streamIndex === undefined &&
			rule.minHeight === undefined && rule.maxHeight === undefined) return null;
		rules.push({
			preset: rule.preset,
			libraryType: rule.libraryType as string | undefined,
			libraryId: rule.libraryId as string | undefined,
			streamIndex: rule.streamIndex as number | undefined,
			minHeight: rule.minHeight as number | undefined,
			maxHeight: rule.maxHeight as number | undefined
		});
	}
	return { default: policy.default, rules };
}

/** Track, resolution, library, type, global; later rows win ties. */
export function resolveEffect(policy: EffectsPolicy, context: EffectContext): EffectPreset {
	let chosen = policy.default;
	let best = -1;
	for (const rule of policy.rules) {
		if (rule.libraryType && rule.libraryType !== context.libraryType) continue;
		if (rule.libraryId && rule.libraryId !== context.libraryId) continue;
		if (rule.streamIndex !== undefined && rule.streamIndex !== context.streamIndex) continue;
		if ((rule.minHeight !== undefined || rule.maxHeight !== undefined) && context.height < 1) continue;
		if (rule.minHeight !== undefined && context.height < rule.minHeight) continue;
		if (rule.maxHeight !== undefined && context.height > rule.maxHeight) continue;
		const score = (rule.streamIndex === undefined ? 0 : 8) +
			(rule.minHeight === undefined && rule.maxHeight === undefined ? 0 : 4) +
			(rule.libraryId ? 2 : 0) + (rule.libraryType ? 1 : 0);
		if (score >= best) {
			chosen = rule.preset;
			best = score;
		}
	}
	return chosen;
}

function storageKey(userId: string): string {
	return `effects.${userId}`;
}

export function loadEffectsPolicy(userId: string): EffectsPolicy {
	try {
		const raw = prefs.get(storageKey(userId));
		return raw ? (validateEffectsPolicy(JSON.parse(raw)) ?? defaultEffectsPolicy()) : defaultEffectsPolicy();
	} catch {
		return defaultEffectsPolicy();
	}
}

export function saveEffectsPolicy(userId: string, policy: EffectsPolicy): boolean {
	const valid = validateEffectsPolicy(policy);
	if (!valid) return false;
	try {
		const encoded = JSON.stringify(valid);
		prefs.set(storageKey(userId), encoded);
		return prefs.get(storageKey(userId)) === encoded;
	} catch {
		return false;
	}
}
