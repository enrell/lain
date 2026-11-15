import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { configureApi } from './client';
import { holdLease, leaseHeld, onLeaseResync, releaseLease, syncLease } from './lease';

/** A stream that stays open until `end()`. */
function stream(): { res: Response; end: () => void } {
	let close = (): void => {};
	const body = new ReadableStream<Uint8Array>({
		start(c) {
			c.enqueue(new TextEncoder().encode('event: ready\ndata: {}\n\n'));
			close = () => c.close();
		}
	});
	return { res: new Response(body, { status: 200 }), end: () => close() };
}

beforeEach(() => configureApi({ getToken: () => 'tok', onUnauthorized: () => {} }));
afterEach(() => {
	releaseLease();
	onLeaseResync(null);
	vi.unstubAllGlobals();
});

describe('the unlock lease (D-294)', () => {
	it('holds one stream while something is unlocked and drops it when nothing is', async () => {
		const calls: { url: string; auth: string | null; signal: AbortSignal }[] = [];
		vi.stubGlobal(
			'fetch',
			vi.fn(async (url: string, init: RequestInit) => {
				calls.push({ url, auth: new Headers(init.headers).get('Authorization'), signal: init.signal! });
				return stream().res;
			})
		);
		syncLease([{ private: true, locked: true }, {}]);
		expect(leaseHeld()).toBe(false);
		syncLease([{ private: true, locked: false }]);
		syncLease([{ private: true, locked: false }]);
		await vi.waitFor(() => expect(calls.length).toBe(1));
		expect(calls[0]).toMatchObject({ url: '/api/events', auth: 'Bearer tok' });
		syncLease([{ private: true, locked: true }]);
		expect(leaseHeld()).toBe(false);
		expect(calls[0].signal.aborted).toBe(true);
	});

	it('reconnects after a drop and says so, and stops on 401', async () => {
		const first = stream();
		const answers = [first.res, stream().res];
		let n = 0;
		vi.stubGlobal(
			'fetch',
			vi.fn(async () => answers[n++] ?? new Response('', { status: 401 }))
		);
		const resynced = vi.fn();
		onLeaseResync(resynced);
		holdLease();
		await vi.waitFor(() => expect(n).toBe(1));
		first.end();
		await vi.waitFor(() => expect(n).toBe(2), { timeout: 3000 });
		await vi.waitFor(() => expect(resynced).toHaveBeenCalledTimes(1));
		releaseLease();

		n = 2;
		holdLease();
		await vi.waitFor(() => expect(leaseHeld()).toBe(false));
	});
});
