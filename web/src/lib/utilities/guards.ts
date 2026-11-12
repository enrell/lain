/*
 * Route decisions as pure functions so they can be unit tested
 * without mounting SvelteKit.
 */

export interface GuardState {
	ready: boolean;
	setupRequired: boolean;
	authenticated: boolean;
	/** True while the post-setup welcome screen is still on display. */
	freshSetup: boolean;
	/** True when the signed-in user has the admin role. */
	admin: boolean;
	pathname: string;
}

/**
 * Returns the path to redirect to, or null to stay. Called from the
 * root layout effect; keeping it pure makes the loop rules explicit.
 */
export function routeRedirect(state: GuardState): string | null {
	const { ready, setupRequired, authenticated, freshSetup, admin, pathname } = state;
	if (!ready) return null;
	if (setupRequired) {
		return pathname === '/setup' ? null : '/setup';
	}
	if (pathname === '/setup') {
		// The welcome step may stay while the guard would otherwise
		// bounce a freshly authenticated admin straight to Home.
		if (freshSetup && authenticated) return null;
		return '/';
	}
	if (!authenticated) {
		return pathname === '/login' ? null : '/login';
	}
	if (pathname === '/login') return '/';
	// A section every account has, never /settings itself: /settings
	// reopens the last section visited in this browser, which can be an
	// admin's, and bouncing between the two never settled.
	// Acquire is not a settings page: an account without it lands on Home,
	// not on its own profile with no word of why.
	if (isAdminRoute(pathname) && !admin) return pathname.startsWith('/settings') ? ACCOUNT_SETTINGS : '/';
	return null;
}

/** A settings section every account has: where a non-admin lands from an admin route. */
export const ACCOUNT_SETTINGS = '/settings/profile';

export const ADMIN_ROUTES = [
	'/settings/libraries',
	'/settings/users',
	'/settings/transcoding',
	'/settings/integrations',
	'/settings/plugins',
	'/settings/backup',
	'/settings/downloads',
	'/settings/indexers',
	'/settings/acquisition',
	'/acquire'
] as const;

export function isAdminRoute(pathname: string): boolean {
	return ADMIN_ROUTES.some((r) => pathname === r || pathname.startsWith(r + '/'));
}

export function canAccessAdmin(role: string | undefined | null): boolean {
	return role === 'admin';
}

/**
 * True when a keyboard event target is a text-entry surface: player
 * shortcuts must never fire while the user is typing. Duck-typed so it
 * stays testable without a DOM.
 */
export function isTypingTarget(target: EventTarget | null): boolean {
	if (target === null || typeof target !== 'object') return false;
	const el = target as { tagName?: unknown; isContentEditable?: unknown };
	const tag = typeof el.tagName === 'string' ? el.tagName : '';
	if (tag === 'INPUT' || tag === 'TEXTAREA' || tag === 'SELECT') return true;
	return el.isContentEditable === true;
}
