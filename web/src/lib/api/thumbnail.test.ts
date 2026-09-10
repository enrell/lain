import { describe, expect, it } from 'vitest';
import { thumbnail } from './thumbnail';

describe('thumbnail.url', () => {
	it('builds the gateway thumbnail URL with token and explicit params', () => {
		const url = thumbnail.url('id/with space', 'tok en/+', { at: 12, width: 320 });
		expect(url.startsWith('/api/items/id%2Fwith%20space/thumbnail?')).toBe(true);
		const q = new URLSearchParams(url.split('?')[1]);
		expect(q.get('token')).toBe('tok en/+');
		expect(q.get('t')).toBe('12');
		expect(q.get('w')).toBe('320');
	});

	it('applies composition defaults and omits a missing token', () => {
		const url = thumbnail.url('abc', null);
		const q = new URLSearchParams(url.split('?')[1]);
		expect(q.get('t')).toBe('10');
		expect(q.get('w')).toBe('480');
		expect(q.has('token')).toBe(false);
	});
});
