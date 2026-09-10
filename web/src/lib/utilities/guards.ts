/*
 * Route decisions as pure functions so they can be unit tested
 * without mounting SvelteKit.
 */

export interface GuardState {
	ready: boolean;
	setupRequired: boolean;
	authenticated: boolean;
	/** True when the signed-in user has the admin role. */
	admin: boolean;
	pathname: string;
}

/**
 * Returns the path to redirect to, or null to stay. Called from the
 * root layout effect; keeping it pure makes the loop rules explicit.
 */
export function routeRedirect(state: GuardState): string | null {
	const { ready, setupRequired, authenticated, admin, pathname } = state;
	if (!ready) return null;
	if (setupRequired) {
		return pathname === '/setup' ? null : '/setup';
	}
	if (pathname === '/setup') return '/';
	if (!authenticated) {
		return pathname === '/login' ? null : '/login';
	}
	if (pathname === '/login') return '/';
	if (isAdminRoute(pathname) && !admin) return '/settings';
	return null;
}

export const ADMIN_ROUTES = [
	'/settings/libraries',
	'/settings/users',
	'/settings/plugins',
	'/settings/backup'
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
