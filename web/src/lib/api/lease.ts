/*
 * The unlock lease (D-294). A private library stays unlocked only while
 * the server sees this session alive: an open `GET /api/events` stream.
 * The server ends the unlock by itself shortly after the last stream of
 * the session closes (tab or browser closed, network gone), so nothing
 * here ever has to tell it to lock, and a request that is dropped on the
 * way can only lock sooner.
 *
 * The stream is held only while something is unlocked: an idle tab
 * keeps no connection open (a browser allows six per server on HTTP/1).
 */
import { currentToken } from './client';

const RETRY_MS = [500, 1000, 2000, 5000];

let controller: AbortController | null = null;
let resync: (() => void) | null = null;

/** Called after the stream came back from a drop: the server may have locked meanwhile. */
export function onLeaseResync(fn: (() => void) | null): void {
	resync = fn;
}

export function leaseHeld(): boolean {
	return controller !== null;
}

/** Opens the stream (once) and keeps it open until `releaseLease`. */
export function holdLease(): void {
	if (controller || typeof fetch !== 'function') return;
	const mine = new AbortController();
	controller = mine;
	void run(mine);
}

export function releaseLease(): void {
	controller?.abort();
	controller = null;
}

/** Holds while any private library is unlocked in this session; releases when none is. */
export function syncLease(libraries: { private?: boolean; locked?: boolean }[]): void {
	if (libraries.some((l) => l.private && l.locked === false)) holdLease();
	else releaseLease();
}

async function run(mine: AbortController): Promise<void> {
	let failures = 0;
	let dropped = false;
	while (!mine.signal.aborted) {
		const token = currentToken();
		if (!token) break;
		try {
			const res = await fetch('/api/events', {
				headers: { Authorization: `Bearer ${token}`, Accept: 'text/event-stream' },
				signal: mine.signal,
				cache: 'no-store'
			});
			if (res.status === 401 || res.status === 403) break;
			if (res.ok && res.body) {
				failures = 0;
				if (dropped) resync?.();
				const reader = res.body.getReader();
				// Nothing is read out of the frames: holding it open is the point.
				while (!(await reader.read()).done);
			}
		} catch {
			// Aborted, or the network: the loop decides.
		}
		if (mine.signal.aborted) break;
		dropped = true;
		const wait = RETRY_MS[Math.min(failures++, RETRY_MS.length - 1)];
		await new Promise((r) => setTimeout(r, wait));
	}
	if (controller === mine) controller = null;
}
