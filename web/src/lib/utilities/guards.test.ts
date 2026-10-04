import { describe, expect, it } from 'vitest';
import { canAccessAdmin, isAdminRoute, isTypingTarget, routeRedirect } from './guards';
import { SECTIONS } from '$lib/settings/sections';

const base = {
	ready: true,
	setupRequired: false,
	authenticated: false,
	freshSetup: false,
	admin: false,
	pathname: '/'
};

describe('routeRedirect', () => {
	it('waits for the boot sequence', () => {
		expect(routeRedirect({ ...base, ready: false })).toBeNull();
	});

	it('forces first-run setup regardless of path', () => {
		expect(routeRedirect({ ...base, setupRequired: true, pathname: '/' })).toBe('/setup');
		expect(routeRedirect({ ...base, setupRequired: true, pathname: '/setup' })).toBeNull();
	});

	it('keeps the welcome step visible right after setup', () => {
		expect(
			routeRedirect({ ...base, authenticated: true, admin: true, freshSetup: true, pathname: '/setup' })
		).toBeNull();
		expect(
			routeRedirect({ ...base, authenticated: true, admin: true, freshSetup: false, pathname: '/setup' })
		).toBe('/');
	});

	it('sends anonymous visitors to login, except on auth pages', () => {
		expect(routeRedirect({ ...base, pathname: '/library' })).toBe('/login');
		expect(routeRedirect({ ...base, pathname: '/login' })).toBeNull();
		// A completed setup never shows the setup screen.
		expect(routeRedirect({ ...base, pathname: '/setup' })).toBe('/');
	});

	it('sends authenticated users away from login', () => {
		expect(
			routeRedirect({ ...base, authenticated: true, admin: true, pathname: '/login' })
		).toBe('/');
	});

	it('keeps normal users out of admin routes', () => {
		const state = { ...base, authenticated: true, admin: false };
		expect(routeRedirect({ ...state, pathname: '/settings' })).toBeNull();
		expect(routeRedirect({ ...state, pathname: '/settings/libraries' })).toBe('/settings');
		expect(routeRedirect({ ...state, pathname: '/settings/users' })).toBe('/settings');
		expect(routeRedirect({ ...state, pathname: '/settings/plugins' })).toBe('/settings');
		expect(routeRedirect({ ...state, pathname: '/settings/backup' })).toBe('/settings');
	});

	it('lets admins into admin routes', () => {
		expect(
			routeRedirect({
				...base,
				authenticated: true,
				admin: true,
				pathname: '/settings/plugins'
			})
		).toBeNull();
	});
});

describe('admin gating helpers', () => {
	it('classifies admin routes', () => {
		expect(isAdminRoute('/settings')).toBe(false);
		expect(isAdminRoute('/settings/libraries')).toBe(true);
		expect(isAdminRoute('/settings/users')).toBe(true);
		expect(isAdminRoute('/settings/plugins')).toBe(true);
		expect(isAdminRoute('/settings/backup')).toBe(true);
		expect(isAdminRoute('/settings/downloads')).toBe(true);
		expect(isAdminRoute('/settings/libraries/extra')).toBe(true);
		expect(isAdminRoute('/settings/offline')).toBe(false);
	});

	// The rail and the guard must agree: a SERVER section the guard does
	// not know renders its page shell to non-admins (the API still 403s).
	it('guards every SERVER settings section and no YOU section', () => {
		for (const s of SECTIONS) {
			expect(isAdminRoute(s.href), s.href).toBe(s.scope === 'server');
		}
	});

	it('only the admin role passes', () => {
		expect(canAccessAdmin('admin')).toBe(true);
		expect(canAccessAdmin('user')).toBe(false);
		expect(canAccessAdmin(undefined)).toBe(false);
	});
});

describe('isTypingTarget', () => {
	it('detects text entry surfaces', () => {
		expect(isTypingTarget(null)).toBe(false);
		const input = { tagName: 'INPUT', isContentEditable: false } as unknown as HTMLElement;
		const div = { tagName: 'DIV', isContentEditable: false } as unknown as HTMLElement;
		expect(isTypingTarget(input)).toBe(true);
		expect(isTypingTarget(div)).toBe(false);
	});
});
