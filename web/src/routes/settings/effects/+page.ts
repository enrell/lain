import { redirect } from '@sveltejs/kit';
import type { PageLoad } from './$types';

// Video effects moved into the personal Playback section.
export const load: PageLoad = () => {
	redirect(307, '/settings/playback#effects');
};
