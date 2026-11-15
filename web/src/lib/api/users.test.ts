import { afterEach, describe, expect, it, vi } from 'vitest';
import { ApiError } from './client';
import { effectiveMembers, libraryAudience, namedLibraries } from './libraries';
import type { Library } from './types';
import { toggleUserLibrary, userMaxAudience, users } from './users';

function lib(over: Partial<Library> = {}): Library {
	return {
		id: 'lib-1',
		name: 'Anime',
		type: 'anime',
		path: '/media/anime',
		source: 'local',
		created_at: 1,
		...over
	};
}

describe('api.users access helpers', () => {
	afterEach(() => {
		vi.unstubAllGlobals();
	});

	it('reads defaults and toggles library allow-lists without dropping hidden ids', () => {
		expect(userMaxAudience({})).toBe('mature');
		expect(userMaxAudience({ max_audience: 'everyone' })).toBe('everyone');
		expect(libraryAudience({})).toBe('everyone');
		expect(libraryAudience({ audience: 'teen' })).toBe('teen');

		// Starting from unset ("all"), toggling off one visible library yields the rest.
		expect(toggleUserLibrary(undefined, ['a', 'b', 'c'], 'b')).toEqual(['a', 'c']);
		// Toggling an id already in an allow-list removes it while preserving others (e.g. hidden).
		expect(toggleUserLibrary(['hidden-priv', 'a'], ['a', 'b'], 'a')).toEqual(['hidden-priv']);
		expect(toggleUserLibrary(['hidden-priv'], ['a', 'b'], 'b')).toEqual(['hidden-priv', 'b']);
	});

	it('filters locked/nameless private libraries and deduplicates owner + members', () => {
		const visible = lib({ id: 'pub', name: 'Shows' });
		const unlocked = lib({ id: 'priv-open', name: 'Open', private: true, locked: false });
		const locked = lib({ id: 'priv-locked', name: '', private: true, locked: true });
		const blank = lib({ id: 'blank', name: '   ' });
		expect(namedLibraries([visible, unlocked, locked, blank]).map((l) => l.id)).toEqual([
			'pub',
			'priv-open'
		]);
		expect(effectiveMembers({ owner: 'u-admin', members: ['u-bob', 'u-admin', 'u-eve'] })).toEqual([
			'u-admin',
			'u-bob',
			'u-eve'
		]);
	});

	it('sends null when clearing libraries and surfaces server errors', async () => {
		const fetchMock = vi.fn().mockResolvedValue(
			new Response(JSON.stringify({ id: 'u2', username: 'kid', role: 'user', disabled: false, pwd_ver: 1, created_at: 1, max_audience: 'everyone' }), {
				status: 200,
				headers: { 'content-type': 'application/json' }
			})
		);
		vi.stubGlobal('fetch', fetchMock);

		const res = await users.patch('u2', { libraries: null, max_audience: 'everyone' });
		expect(res.max_audience).toBe('everyone');
		const [url, init] = fetchMock.mock.calls[0];
		expect(url).toBe('/api/users/u2');
		expect(init.method).toBe('PATCH');
		expect(JSON.parse(init.body)).toEqual({ libraries: null, max_audience: 'everyone' });

		fetchMock.mockResolvedValueOnce(
			new Response(JSON.stringify({ error: 'unknown library lib-x' }), {
				status: 400,
				headers: { 'content-type': 'application/json' }
			})
		);
		await expect(users.patch('u2', { libraries: ['lib-x'] })).rejects.toThrowError(ApiError);
	});
});
