import { acquire } from './acquire';
import { auth } from './auth';
import { backup } from './backup';
import { catalog } from './catalog';
import { downloads } from './downloads';
import { enrich } from './enrich';
import { items } from './items';
import { libraries } from './libraries';
import { integrations, links, list } from './list';
import { localplay } from './localplay';
import { me } from './me';
import { playback } from './playback';
import { plugins } from './plugins';
import { progress } from './progress';
import { reader } from './reader';
import { search } from './search';
import { social } from './social';
import { thumbnail } from './thumbnail';
import { theme } from './theme';
import { transcodeAdmin } from './transcode-settings';
import { users } from './users';

/**
 * Typed access to the Lain gateway. Components import this object (or
 * a domain module directly) — never fetch() by hand, never build auth
 * headers outside the client.
 */
export const api = {
	acquire,
	auth,
	backup,
	catalog,
	downloads,
	enrich,
	items,
	libraries,
	integrations,
	links,
	list,
	localplay,
	me,
	playback,
	plugins,
	progress,
	reader,
	search,
	social,
	thumbnail,
	theme,
	transcodeAdmin,
	users
};

export { ApiError, buildUrl, mediaUrl } from './client';
export * from './types';
export type * from './social';
export { targetOf } from './social';
export type * from './acquire';
