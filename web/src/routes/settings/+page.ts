import { redirect } from '@sveltejs/kit';
import { session } from '$lib/auth/session.svelte';
import { prefs } from '$lib/auth/storage';
import { sectionFor } from '$lib/settings/sections';
import { isAdminRoute, ACCOUNT_SETTINGS } from '$lib/utilities/guards';
import type { PageLoad } from './$types';

// /settings has no page of its own: it reopens the last section visited
// (the admin guard still applies there). The list-link OAuth callback
// used to land here with ?linked=/?link_error=, so those go to Connections.
// The last section is this browser's, not this account's: an admin's
// section is skipped for an account known not to be one, rather than
// opened only for the guard to send it away again.
import { resolveLoadUrl } from '$lib/demo/url';

export const load: PageLoad = ({ url }) => {
	const resolved = resolveLoadUrl(url);
	const oauth = resolved.searchParams.has('linked') || resolved.searchParams.has('link_error');
	const last = prefs.get('settings.last');
	const lastAllowed = last && sectionFor(last) && !(session.authenticated && !session.isAdmin && isAdminRoute(last));
	const target = oauth ? '/settings/connections' : lastAllowed ? last : ACCOUNT_SETTINGS;
	redirect(307, target + resolved.search);
};
