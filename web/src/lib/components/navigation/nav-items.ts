import type { Key } from '$lib/i18n';

/** One bottom-bar destination; the label is an i18n key. */
export interface NavEntry {
	href: string;
	label: Key;
	exact: boolean;
	/** Only admins see it (acquisition is admin-only, D-116). */
	admin?: boolean;
}

export const MOBILE_NAV: NavEntry[] = [
	{ href: '/', label: 'nav.home', exact: true },
	{ href: '/library', label: 'nav.library', exact: false },
	{ href: '/favorites', label: 'nav.favorites', exact: false },
	{ href: '/list', label: 'nav.list', exact: false },
	{ href: '/social', label: 'nav.social', exact: false },
	{ href: '/search', label: 'nav.search', exact: false },
	{ href: '/acquire', label: 'nav.acquire', exact: false, admin: true },
	{ href: '/settings', label: 'nav.settings', exact: false }
];

/**
 * The phone's bottom bar: the places used every day; the rest sit under
 * More. Search stays here: a phone's top bar has no room for the field
 * the desktop header holds, and the bar is where the thumb already is.
 */
export const MOBILE_BAR = ['/', '/library', '/search', '/favorites'];

/** The entries a role sees, in order. */
export function visibleNav(entries: NavEntry[], isAdmin: boolean): NavEntry[] {
	return entries.filter((e) => isAdmin || !e.admin);
}
