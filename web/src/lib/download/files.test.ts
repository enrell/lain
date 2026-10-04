import { describe, expect, it } from 'vitest';
import { downloadable, downloadUrl } from './files';

describe('file downloads', () => {
	it('asks the stream endpoint for an attachment', () => {
		expect(downloadUrl('a b', 'tok')).toBe('/api/items/a%20b/stream?download=1&token=tok');
		expect(downloadUrl('x', null)).toBe('/api/items/x/stream?download=1');
	});
	it('skips missing files and duplicates', () => {
		const got = downloadable([{ id: 'a' }, { id: 'b', missing: true }, { id: 'a' }, { id: 'c' }]);
		expect(got.map((i) => i.id)).toEqual(['a', 'c']);
	});
});
