import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { leaseHeld, releaseLease } from './lease';
import { configureApi } from './client';
import { libraries, shown } from './libraries';

function ok(body: unknown): Response {
	return new Response(JSON.stringify(body), { status: 200, headers: { 'Content-Type': 'application/json' } });
}

beforeEach(() => configureApi({ getToken: () => 'tok', onUnauthorized: () => {} }));
afterEach(() => releaseLease());

describe('private libraries and roots (lain-rs)', () => {
	it('sends the unlock secret, the patch and the PIN to their routes', async () => {
		const calls: { url: string; method: string; body: unknown }[] = [];
		vi.stubGlobal(
			'fetch',
			vi.fn(async (url: string, init?: RequestInit) => {
				// The unlock lease's stream (D-294) is not one of the routes under test.
				if (url === '/api/events') return new Response(new ReadableStream(), { status: 200 });
				calls.push({ url, method: init?.method ?? 'GET', body: init?.body ? JSON.parse(String(init.body)) : null });
				return ok({ id: 'lib-1', locked: false, set: true, roots: [] });
			})
		);
		await libraries.unlock('lib 1', { pin: '2468' });
		expect(leaseHeld()).toBe(true);
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

	it('sends the PIN with a change or removal of a locked library (D-293)', async () => {
		const calls: { method: string; body: unknown }[] = [];
		vi.stubGlobal(
			'fetch',
			vi.fn(async (_url: string, init?: RequestInit) => {
				calls.push({ method: init?.method ?? 'GET', body: init?.body ? JSON.parse(String(init.body)) : null });
				return ok({ id: 'lib-1', private: true, locked: true, name: '', path: '' });
			})
		);
		const lib = await libraries.update('lib-1', { private: false }, { pin: '2468' });
		await libraries.remove('lib-1', { password: 'pw' });
		await libraries.remove('lib-1');
		expect(calls[0].body).toEqual({ private: false, pin: '2468' });
		expect(calls[1]).toEqual({ method: 'DELETE', body: { password: 'pw' } });
		expect(calls[2]).toEqual({ method: 'DELETE', body: null });
		expect(lib.name).toBe('Private library');
	});

	it('names a locked library neutrally and leaves the rest alone', () => {
		const base = { id: 'l', type: 'auto', path: '', source: 'fs', created_at: 0 };
		expect(shown({ ...base, name: '', private: true, locked: true }).name).toBe('Private library');
		expect(shown({ ...base, name: 'Anime', private: true, locked: false }).name).toBe('Anime');
		expect(shown({ ...base, name: 'Anime' }).name).toBe('Anime');
	});
});
