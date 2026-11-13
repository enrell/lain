import { redirect } from '@sveltejs/kit';
import type { PageLoad } from './$types';

// The header calls the list page Tracking; an address typed after its
// label lands there instead of on a dead end.
import { resolveLoadUrl } from '$lib/demo/url';

export const load: PageLoad = ({ url }) => {
	redirect(308, `/list${resolveLoadUrl(url).search}`);
};
