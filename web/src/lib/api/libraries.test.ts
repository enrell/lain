import { beforeEach, describe, expect, it, vi } from 'vitest';
import { configureApi } from './client';
import { libraries } from './libraries';

function ok(body: unknown): Response {
	return new Response(JSON.stringify(body), { status: 200, headers: { 'Content-Type': 'application/json' } });
}

beforeEach(() => configureApi({ getToken: () => 'tok', onUnauthorized: () => {} }));

describe('private libraries and roots (lain-rs)', () => {
	it('sends the unlock secret, the patch and the PIN to their routes', async () => {
		const calls: { url: string; method: string; body: unknown }[] = [];
		vi.stubGlobal(
			'fetch',
			vi.fn(async (url: string, init?: RequestInit) => {
				calls.push({ url, method: init?.method ?? 'GET', body: init?.body ? JSON.parse(String(init.body)) : null });
				return ok({ id: 'lib-1', locked: false, set: true, roots: [] });
			})
		);
		await libraries.unlock('lib 1', { pin: '2468' });
		await libraries.lock('lib-1');
		await libraries.update('lib-1', { private: true });
		await libraries.pin.set('2468', 'secret-pass');
		await libraries.roots();
		expect(calls.map((c) => `${c.method} ${c.url}`)).toEqual([
			'POST /api/libraries/lib%201/unlock',
			'POST /api/libraries/lib-1/lock',
			'PATCH /api/libraries/lib-1',
			'PUT /api/me/pin',
			'GET /api/roots'
		]);
		expect(calls[0].body).toEqual({ pin: '2468' });
		expect(calls[2].body).toEqual({ private: true });
		expect(calls[3].body).toEqual({ pin: '2468', password: 'secret-pass' });
	});
});
