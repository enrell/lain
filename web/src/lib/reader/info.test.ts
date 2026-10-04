import { describe, expect, it } from 'vitest';
import { directionSource, infoRows } from './info';

describe('ComicInfo rows', () => {
	it('lists only what the archive says, in a fixed order', () => {
		expect(infoRows(null)).toEqual([]);
		expect(infoRows({ year: 2021, writer: 'Writer A', series: 'Frieren', volume: 0, publisher: '' })).toEqual([
			{ field: 'series', value: 'Frieren' },
			{ field: 'writer', value: 'Writer A' },
			{ field: 'year', value: '2021' }
		]);
	});
	it('shows the issue count next to the number when known', () => {
		expect(infoRows({ number: '12', count: 40 })).toEqual([{ field: 'number', value: '12 / 40' }]);
		expect(infoRows({ count: 40 })).toEqual([]);
	});
	it('names where the direction comes from', () => {
		expect(directionSource({ direction: 'rtl' })).toBe('archive');
		expect(directionSource({ series: 'x' })).toBe('library');
		expect(directionSource(undefined)).toBe('library');
	});
});
