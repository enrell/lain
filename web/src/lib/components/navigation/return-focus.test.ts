import { describe, expect, it } from 'vitest';
import { linkTarget } from './return-focus';

describe('linkTarget', () => {
	const base = 'http://lain.test/library?view=all';
	it('compares path and query, as the router does', () => {
		expect(linkTarget('/item/abc', base)).toBe('/item/abc');
		expect(linkTarget('/search?q=night', base)).toBe('/search?q=night');
		expect(linkTarget('/item/abc#episodes', base)).toBe('/item/abc');
	});
	it('resolves a relative link against the page', () => {
		expect(linkTarget('?view=shelves', base)).toBe('/library?view=shelves');
		expect(linkTarget('lib-1', 'http://lain.test/library/')).toBe('/library/lib-1');
	});
	it('gives nothing for an address it cannot read', () => {
		expect(linkTarget('http://[bad', base)).toBe('');
	});
});
