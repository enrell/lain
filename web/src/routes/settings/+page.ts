import { redirect } from '@sveltejs/kit';
import { prefs } from '$lib/auth/storage';
import { sectionFor } from '$lib/settings/sections';
import type { PageLoad } from './$types';

// /settings has no page of its own: it reopens the last section visited
// (the admin guard still applies there). The list-link OAuth callback
// used to land here with ?linked=/?link_error=, so those go to Connections.
export const load: PageLoad = ({ url }) => {
	const oauth = url.searchParams.has('linked') || url.searchParams.has('link_error');
	const last = prefs.get('settings.last');
	const target = oauth ? '/settings/connections' : last && sectionFor(last) ? last : '/settings/profile';
	redirect(307, target + url.search);
};
