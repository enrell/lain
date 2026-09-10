import { beforeEach, describe, expect, it } from 'vitest';
import { session } from './session.svelte';

function fakeUser(role: 'admin' | 'user') {
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
		session.user = fakeUser('user');
		expect(session.isAdmin).toBe(false);
	});

	it('logout clears token and user in one place', () => {
		session.token = 'tok';
		session.user = fakeUser('admin');
		session.logout();
		expect(session.token).toBeNull();
		expect(session.user).toBeNull();
		expect(session.authenticated).toBe(false);
	});
});
