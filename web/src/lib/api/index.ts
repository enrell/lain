import { auth } from './auth';
import { backup } from './backup';
import { catalog } from './catalog';
import { enrich } from './enrich';
import { libraries } from './libraries';
import { me } from './me';
import { playback } from './playback';
import { plugins } from './plugins';
import { progress } from './progress';
import { search } from './search';
import { thumbnail } from './thumbnail';
import { users } from './users';

/**
 * Typed access to the Lain gateway. Components import this object (or
 * a domain module directly) — never fetch() by hand, never build auth
 * headers outside the client.
 */
export const api = {
	auth,
	backup,
	catalog,
	enrich,
	libraries,
	me,
	playback,
	plugins,
	progress,
	search,
	thumbnail,
	users
};

export { ApiError, buildUrl, mediaUrl } from './client';
export * from './types';
