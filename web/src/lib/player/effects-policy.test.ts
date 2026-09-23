import { afterEach, describe, expect, it, vi } from 'vitest';
import { defaultEffectsPolicy, loadEffectsPolicy, resolveEffect, saveEffectsPolicy, validateEffectsPolicy } from './effects-policy';

afterEach(() => vi.unstubAllGlobals());

describe('video effects policy', () => {
	it('starts disabled and resolves the most specific matching rule', () => {
		const policy = {
			...defaultEffectsPolicy(),
			default: 'anime4k-a' as const,
			rules: [
			{ libraryType: 'anime', preset: 'anime4k-aa' as const },
			{ libraryId: 'library-a', preset: 'anime4k-lite' as const },
			{ libraryId: 'library-a', minHeight: 1080, preset: 'off' as const },
			{ libraryId: 'library-a', minHeight: 1080, streamIndex: 2, preset: 'anime4k-dog-x2' as const }
			]
		};
		expect(resolveEffect(defaultEffectsPolicy(), { libraryId: 'x', libraryType: 'anime', height: 720, streamIndex: 0 })).toBe('off');
		expect(resolveEffect(policy, { libraryId: 'x', libraryType: 'movie', height: 720, streamIndex: 0 })).toBe('anime4k-a');
		expect(resolveEffect(policy, { libraryId: 'x', libraryType: 'anime', height: 720, streamIndex: 0 })).toBe('anime4k-aa');
		expect(resolveEffect(policy, { libraryId: 'library-a', libraryType: 'anime', height: 720, streamIndex: 0 })).toBe('anime4k-lite');
		expect(resolveEffect(policy, { libraryId: 'library-a', libraryType: 'anime', height: 1080, streamIndex: 0 })).toBe('off');
		expect(resolveEffect(policy, { libraryId: 'library-a', libraryType: 'anime', height: 0, streamIndex: 0 })).toBe('anime4k-lite');
		expect(resolveEffect(policy, { libraryId: 'library-a', libraryType: 'anime', height: 1080, streamIndex: 2 })).toBe('anime4k-dog-x2');
	});

	it('rejects malformed saved policies instead of applying arbitrary values', () => {
		expect(validateEffectsPolicy({ default: 'unknown', rules: [] })).toBeNull();
		expect(validateEffectsPolicy({ default: 'off', rules: [{ minHeight: -1, preset: 'anime4k-a' }] })).toBeNull();
		expect(validateEffectsPolicy({ default: 'off', rules: [{ preset: 'anime4k-a' }] })).toBeNull();
		expect(validateEffectsPolicy({ default: 'off', rules: [{ libraryType: 'anime', preset: 'anime4k-lite' }] })?.rules).toHaveLength(1);
	});

	it('persists account-scoped defaults in browser storage', () => {
		const values = new Map<string, string>();
		vi.stubGlobal('localStorage', {
			getItem: (key: string) => values.get(key) ?? null,
			setItem: (key: string, value: string) => { values.set(key, value); }
		});
		const policy = { default: 'anime4k-lite' as const, rules: [{ libraryType: 'anime', preset: 'anime4k-a' as const }] };
		expect(saveEffectsPolicy('user-a', policy)).toBe(true);
		expect(loadEffectsPolicy('user-a')).toEqual(policy);
		expect(loadEffectsPolicy('user-b')).toEqual(defaultEffectsPolicy());
	});
});
