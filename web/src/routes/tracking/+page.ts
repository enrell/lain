import { redirect } from '@sveltejs/kit';
import type { PageLoad } from './$types';

// The header calls the list page Tracking; an address typed after its
// label lands there instead of on a dead end.
export const load: PageLoad = ({ url }) => {
	redirect(308, `/list${url.search}`);
};
