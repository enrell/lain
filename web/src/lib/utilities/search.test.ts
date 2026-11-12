import { describe, expect, it } from 'vitest';
import { parseRecent, searchHref, withRecent } from './search';

describe('searchHref', () => {
	it('leads a query to its results, trimmed and escaped', () => {
		expect(searchHref('  paper moon ')).toBe('/search?q=paper%20moon');
		expect(searchHref('a&b')).toBe('/search?q=a%26b');
	});
	it('leads no query to the bare page', () => {
		expect(searchHref('')).toBe('/search');
		expect(searchHref('   ')).toBe('/search');
	});
});

describe('withRecent', () => {
	it('puts the newest first and keeps one of each, whatever the case', () => {
		expect(withRecent(['night', 'harbor'], 'Harbor')).toEqual(['Harbor', 'night']);
	});
	it('ignores half words', () => {
		expect(withRecent(['night'], ' a ')).toEqual(['night']);
	});
	it('keeps only the limit', () => {
		expect(withRecent(['a1', 'b2', 'c3'], 'd4', 3)).toEqual(['d4', 'a1', 'b2']);
	});
});

describe('parseRecent', () => {
	it('reads a kept list', () => {
		expect(parseRecent('["night","harbor"]')).toEqual(['night', 'harbor']);
	});
	it('drops what is not a string and what is past the limit', () => {
		expect(parseRecent('["a1", 3, null, "b2", "c3"]', 2)).toEqual(['a1', 'b2']);
	});
	it('reads nothing from nothing or from garbage', () => {
		expect(parseRecent(null)).toEqual([]);
		expect(parseRecent('{not json')).toEqual([]);
		expect(parseRecent('{"a":1}')).toEqual([]);
	});
});
