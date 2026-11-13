import { describe, expect, it } from 'vitest';
import { hashed, unwrapHashUrl } from './url';

describe('demo hash URL helpers', () => {
	const basePath = '/lain/demo';

	describe('unwrapHashUrl', () => {
		it('extracts route pathname, query, and hash fragment from hash-routed URLs', () => {
			const url = unwrapHashUrl(new URL('https://example.com/lain/demo/#/library?view=all#top'), basePath);
			expect(url.pathname).toBe('/library');
			expect(url.search).toBe('?view=all');
			expect(url.searchParams.get('view')).toBe('all');
			expect(url.hash).toBe('#top');
		});

		it('reads query params on search and deep routes', () => {
			const search = unwrapHashUrl(new URL('https://example.com/lain/demo/#/search?q=pepper%20and%20carrot'), basePath);
			expect(search.pathname).toBe('/search');
			expect(search.searchParams.get('q')).toBe('pepper and carrot');

			const player = unwrapHashUrl(new URL('https://example.com/lain/demo/#/player/abc123?t=15'), basePath);
			expect(player.pathname).toBe('/player/abc123');
			expect(player.searchParams.get('t')).toBe('15');
		});

		it('resolves the mount root without hash to logical root /', () => {
			expect(unwrapHashUrl(new URL('https://example.com/lain/demo/'), basePath).pathname).toBe('/');
			expect(unwrapHashUrl(new URL('https://example.com/lain/demo'), basePath).pathname).toBe('/');
			expect(unwrapHashUrl(new URL('https://example.com/lain/demo/#main'), basePath).pathname).toBe('/');
		});

		it('preserves null, undefined, and placeholder URLs before hydration', () => {
			const placeholder = new URL('a:');
			expect(unwrapHashUrl(placeholder, basePath)).toBe(placeholder);
			expect(unwrapHashUrl(null, basePath)).toBeNull();
			expect(unwrapHashUrl(undefined, basePath)).toBeUndefined();
		});
	});

	describe('hashed', () => {
		const current = new URL('https://example.com/lain/demo/#/library?view=all');

		it('rewrites root-relative paths and queries onto the hash router', () => {
			expect(hashed('/settings/profile', current, basePath)).toBe('#/settings/profile');
			expect(hashed('/settings/playback#effects', current, basePath)).toBe('#/settings/playback#effects');
			expect(hashed('?sort=title', current, basePath)).toBe('#/library?sort=title');
			expect(hashed('#shelf-shows', current, basePath)).toBe('#/library?view=all#shelf-shows');
			expect(hashed('#/search?q=s', current, basePath)).toBe('#/search?q=s');
		});

		it('does not rewrite external or protocol-relative targets', () => {
			expect(hashed('//example.org/test', current, basePath)).toBe('//example.org/test');
			expect(hashed('https://example.org/apk', current, basePath)).toBe('https://example.org/apk');
			const extUrl = new URL('https://example.org/other');
			expect(hashed(extUrl, current, basePath)).toBe(extUrl);
		});

		it('rewrites same-origin URL instances derived from unwrapped page.url', () => {
			const pageUrl = unwrapHashUrl(current, basePath);
			pageUrl.searchParams.set('sort', 'recent');
			expect(hashed(pageUrl, current, basePath)).toBe('#/library?view=all&sort=recent');
		});
	});
});
