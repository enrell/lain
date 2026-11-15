import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { session } from './session.svelte';

function fakeUser(role: string) {
	return {
		id: 'user-test',
		username: 'tester',
		role,
		disabled: false,
		pwd_ver: 1,
		created_at: 0
	};
}

beforeEach(() => {
	session.token = null;
	session.user = null;
});

describe('session state', () => {
	it('reports authentication only when a user is loaded', () => {
		expect(session.authenticated).toBe(false);
		session.token = 'tok';
		expect(session.authenticated).toBe(false);
		session.user = fakeUser('user');
		expect(session.authenticated).toBe(true);
	});

	it('derives admin access from the live role', () => {
		session.user = fakeUser('admin');
		expect(session.isAdmin).toBe(true);
		expect(session.canManage).toBe(true);
		session.user = fakeUser('manager');
		expect(session.isAdmin).toBe(false);
		expect(session.isManager).toBe(true);
		expect(session.canManage).toBe(true);
		session.user = fakeUser('user');
		expect(session.isAdmin).toBe(false);
		expect(session.isManager).toBe(false);
		expect(session.canManage).toBe(false);
		session.user = fakeUser('root');
		expect(session.isAdmin).toBe(false);
		expect(session.canManage).toBe(false);
	});

	it('logout clears token and user in one place', () => {
		session.token = 'tok';
		session.user = fakeUser('admin');
		session.logout();
		expect(session.token).toBeNull();
		expect(session.user).toBeNull();
		expect(session.authenticated).toBe(false);
	});

	it('logout ends the session on the server with the token it just dropped', async () => {
		const calls: { url: string; init: RequestInit }[] = [];
		vi.stubGlobal('fetch', (url: string, init: RequestInit) => {
			calls.push({ url, init });
			return Promise.resolve(new Response('{"signed_out":true}', { status: 200 }));
		});
		session.token = 'tok-1';
		session.user = fakeUser('user');
		session.logout();
		expect(session.token).toBeNull();
		expect(calls).toHaveLength(1);
		expect(calls[0].url).toBe('/api/auth/logout');
		expect(calls[0].init.method).toBe('POST');
		expect(new Headers(calls[0].init.headers).get('Authorization')).toBe('Bearer tok-1');
		// A 401 clears the session without calling the server again.
		session.token = 'tok-2';
		session.invalidate();
		expect(calls).toHaveLength(1);
	});

	it('logout survives a server that cannot be reached', async () => {
		vi.stubGlobal('fetch', () => Promise.reject(new TypeError('offline')));
		session.token = 'tok';
		session.user = fakeUser('user');
		session.logout();
		await Promise.resolve();
		expect(session.authenticated).toBe(false);
	});

	it('logout remembers the account and never the token', () => {
		const values = new Map<string, string>();
		vi.stubGlobal('localStorage', {
			getItem: (key: string) => values.get(key) ?? null,
			setItem: (key: string, value: string) => {
				values.set(key, value);
			},
			removeItem: (key: string) => {
				values.delete(key);
			}
		});
		session.token = 'secret-token';
		session.user = {
			...fakeUser('manager'),
			profile: { display_name: 'Mina', avatar: { kind: 'upload', version: 4 } }
		};
		session.logout();
		const raw = values.get('lain.accounts') ?? '';
		expect(raw).not.toContain('secret-token');
		expect(raw).not.toMatch(/token|password|pin|version/i);
		expect(JSON.parse(raw)).toEqual([
			{ id: 'user-test', username: 'tester', displayName: 'Mina', avatar: { kind: 'upload' } }
		]);
		expect(values.has('lain.token')).toBe(false);
		expect(session.user).toBeNull();
	});
});

afterEach(() => vi.unstubAllGlobals());
