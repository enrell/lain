import { describe, expect, it } from 'vitest';
import { cacheKey, classify, itemKeys } from './routes';

const origin = 'https://lain.test';
const assets = new Set(['/_app/immutable/entry/app.js', '/favicon.svg']);
const c = (path: string, method = 'GET', mode = 'cors', o = origin) =>
	classify(new URL(path, o), method, mode, origin, assets);

describe('offline routes', () => {
	it('touches only same-origin GETs', () => {
		expect(c('/api/items/x/pages', 'PUT')).toBe('pass');
		expect(c('/api/items/x/progress', 'PUT')).toBe('pass');
		expect(c('https://cdn.test/x.js', 'GET', 'cors', 'https://cdn.test')).toBe('pass');
	});
	it('routes navigations, assets, session and saved reads', () => {
		expect(c('/read/abc', 'GET', 'navigate')).toBe('navigate');
		expect(c('/_app/immutable/entry/app.js')).toBe('asset');
		expect(c('/api/me')).toBe('session');
		expect(c('/api/setup/status')).toBe('session');
		for (const p of ['/api/items/a/pages', '/api/items/a/pages/3', '/api/items/a/progress', '/api/catalog/a', '/api/catalog/a/episodes']) {
			expect(c(p), p).toBe('saved');
		}
	});
	it('never serves streams, transcodes, lists or writes from cache', () => {
		for (const p of ['/api/items/a/stream', '/api/items/a/transcode', '/api/catalog', '/api/search', '/api/downloads', '/api/items/a/pages/x']) {
			expect(c(p), p).toBe('pass');
		}
	});
	it('drops the token from cache keys and keeps other params', () => {
		expect(cacheKey(new URL('/api/items/a/pages/2?token=abc', origin))).toBe('/api/items/a/pages/2');
		expect(cacheKey(new URL('/api/x?b=1&token=t&a=2', origin))).toBe('/api/x?b=1&a=2');
	});
	it('lists every key a saved item uses', () => {
		const keys = itemKeys('a b', 2);
		expect(keys).toContain('/api/items/a%20b/pages/1');
		expect(keys).toContain('/api/catalog/a%20b/episodes');
		expect(keys).toHaveLength(6);
	});
});
