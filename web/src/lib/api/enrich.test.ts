import { beforeEach, describe, expect, it, vi } from 'vitest';
import { configureApi } from './client';
import { ENRICH_BATCH, enrich } from './enrich';

beforeEach(() => configureApi({ getToken: () => 'tok', onUnauthorized: () => {} }));

describe('enrich.batch', () => {
	it('reads a list longer than the server takes in several requests', async () => {
		const asked: string[][] = [];
		vi.stubGlobal(
			'fetch',
			vi.fn(async (url: string) => {
				const ids = new URLSearchParams(url.split('?')[1]).get('ids')!.split(',');
				asked.push(ids);
				// The server refuses more than it takes, as /api/enrichments does.
				if (ids.length > ENRICH_BATCH) return new Response('{"error":"too many ids"}', { status: 400 });
				const items = ids.map((id) => ({ item_id: id, title: id }));
				return new Response(JSON.stringify({ items }), { status: 200, headers: { 'Content-Type': 'application/json' } });
			})
		);
		const ids = Array.from({ length: ENRICH_BATCH * 2 + 1 }, (_, i) => `ep${i}`);
		const found = await enrich.batch(ids);
		expect(asked.map((part) => part.length)).toEqual([ENRICH_BATCH, ENRICH_BATCH, 1]);
		expect(found.map((e) => e.item_id)).toEqual(ids);
		vi.unstubAllGlobals();
	});
});
