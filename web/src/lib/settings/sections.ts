import { t, type Messages } from '$lib/i18n';

/*
 * One source for settings navigation (the rail, the `g`+letter chords)
 * and the command palette index. A setting that exists in the UI but not
 * here is unfindable from Ctrl+K, so rows and entries move together.
 */

export type SettingsScope = 'you' | 'server';

export interface SettingsSection {
	href: string;
	/** Message id; the label is `settings.section.<id>.label`, the hint `.hint`. */
	id: SectionId;
	scope: SettingsScope;
	/** Second key of the `g` chord: `g p` opens Profile. Stable across roles and languages. */
	key: string;
}

export type SectionId = keyof Messages['settings']['section'];

export const sectionLabel = (s: SettingsSection): string => t(`settings.section.${s.id}.label`);
export const sectionHint = (s: SettingsSection): string => t(`settings.section.${s.id}.hint`);

export const SECTIONS: SettingsSection[] = [
	{ href: '/settings/profile', id: 'profile', scope: 'you', key: 'p' },
	{ href: '/settings/playback', id: 'playback', scope: 'you', key: 'y' },
	{ href: '/settings/connections', id: 'connections', scope: 'you', key: 'c' },
	{ href: '/settings/security', id: 'security', scope: 'you', key: 's' },
	{ href: '/settings/privacy', id: 'privacy', scope: 'you', key: 'v' },
	{ href: '/settings/libraries', id: 'libraries', scope: 'server', key: 'l' },
	{ href: '/settings/users', id: 'users', scope: 'server', key: 'u' },
	{ href: '/settings/transcoding', id: 'transcoding', scope: 'server', key: 't' },
	{ href: '/settings/integrations', id: 'integrations', scope: 'server', key: 'i' },
	{ href: '/settings/plugins', id: 'plugins', scope: 'server', key: 'x' },
	{ href: '/settings/backup', id: 'backup', scope: 'server', key: 'b' },
	{ href: '/settings/downloads', id: 'downloads', scope: 'server', key: 'd' },
	{ href: '/settings/indexers', id: 'indexers', scope: 'server', key: 'n' },
	{ href: '/settings/acquisition', id: 'acquisition', scope: 'server', key: 'a' },
	// Kept last (not next to Security) so it does not touch lines the
	// social slice edits; the rail groups by scope, not array order.
	{ href: '/settings/offline', id: 'offline', scope: 'you', key: 'o' }
];

export function visibleSections(admin: boolean): SettingsSection[] {
	return SECTIONS.filter((s) => admin || s.scope === 'you');
}

export function sectionFor(pathname: string): SettingsSection | undefined {
	return SECTIONS.find((s) => pathname === s.href || pathname.startsWith(s.href + '/'));
}

/** A single setting the palette can jump to. `anchor` is a row id on the page. */
export interface SettingEntry {
	/** Message id under `settings.entry`. */
	id: keyof Messages['settings']['entry'];
	href: string;
	anchor: string;
	/** English search aliases: they work in every language (developers type them). */
	keywords: string;
	admin?: boolean;
}

export const entryLabel = (e: SettingEntry): string => t(`settings.entry.${e.id}`);

export const SETTING_ENTRIES: SettingEntry[] = [
	{ id: 'displayName', href: '/settings/profile', anchor: 'display-name', keywords: 'nickname name profile' },
	{ id: 'bio', href: '/settings/profile', anchor: 'bio', keywords: 'about description profile' },
	{ id: 'avatar', href: '/settings/profile', anchor: 'avatar', keywords: 'picture photo mascot icon profile' },
	{ id: 'language', href: '/settings/profile', anchor: 'interface', keywords: 'language locale idioma translation i18n interface' },
	{ id: 'signOut', href: '/settings/profile', anchor: 'account', keywords: 'logout log out session' },
	{ id: 'player', href: '/settings/playback', anchor: 'player', keywords: 'mpv vlc browser external play' },
	{ id: 'playbackLanguage', href: '/settings/playback', anchor: 'language', keywords: 'idioma audio subtitle captions language dub' },
	{ id: 'effects', href: '/settings/playback', anchor: 'effects', keywords: 'anime4k upscale shader webgpu effect' },
	{ id: 'anilist', href: '/settings/connections', anchor: 'anilist', keywords: 'anilist list sync scrobble tracker connect' },
	{ id: 'password', href: '/settings/security', anchor: 'password', keywords: 'password security credentials' },
	{ id: 'privacyProfile', href: '/settings/privacy', anchor: 'privacy-profile', keywords: 'privacy visibility profile public friends private social' },
	{ id: 'privacyActivity', href: '/settings/privacy', anchor: 'privacy-activity', keywords: 'privacy activity feed watching reading social' },
	{ id: 'privacyRatings', href: '/settings/privacy', anchor: 'privacy-ratings', keywords: 'privacy ratings reviews score social' },
	{ id: 'discoverable', href: '/settings/privacy', anchor: 'privacy-discoverable', keywords: 'search find discoverable hidden friends social' },
	{ id: 'friendRequests', href: '/settings/privacy', anchor: 'privacy-requests', keywords: 'friend requests block social' },
	{ id: 'favorites', href: '/settings/privacy', anchor: 'privacy-favorites', keywords: 'favorites favourite profile social' },
	{ id: 'libraries', href: '/settings/libraries', anchor: 'libraries', keywords: 'folder path library add remove root', admin: true },
	{ id: 'scan', href: '/settings/libraries', anchor: 'scan', keywords: 'scan rescan index refresh', admin: true },
	{ id: 'users', href: '/settings/users', anchor: 'users', keywords: 'accounts roles admin disable reset password', admin: true },
	{ id: 'mode', href: '/settings/transcoding', anchor: 'mode', keywords: 'auto custom policy transcode', admin: true },
	{ id: 'delivery', href: '/settings/transcoding', anchor: 'delivery', keywords: 'hls segment timeout throttle progressive delivery stream', admin: true },
	{ id: 'encoding', href: '/settings/transcoding', anchor: 'encoding', keywords: 'crf preset hevc av1 h264 quality ladder bitrate deinterlace', admin: true },
	{ id: 'hardware', href: '/settings/transcoding', anchor: 'hardware', keywords: 'gpu vaapi nvenc qsv hardware decode encode', admin: true },
	{ id: 'processing', href: '/settings/transcoding', anchor: 'processing', keywords: 'hdr tone mapping luminance', admin: true },
	{ id: 'audio', href: '/settings/transcoding', anchor: 'audio', keywords: 'audio bitrate downmix subtitle burn font', admin: true },
	{ id: 'resources', href: '/settings/transcoding', anchor: 'resources', keywords: 'cache threads queue storage path ffmpeg', admin: true },
	{ id: 'sessions', href: '/settings/transcoding', anchor: 'sessions', keywords: 'sessions running jobs cancel', admin: true },
	{ id: 'integrations', href: '/settings/integrations', anchor: 'integrations', keywords: 'oauth client id secret anilist', admin: true },
	{ id: 'plugins', href: '/settings/plugins', anchor: 'capabilities', keywords: 'plugin provider capability composition replace', admin: true },
	{ id: 'backup', href: '/settings/backup', anchor: 'backup', keywords: 'backup snapshot database export', admin: true },
	{ id: 'downloadQueue', href: '/settings/downloads', anchor: 'queue', keywords: 'download fetch url queue pause resume cancel', admin: true },
	{ id: 'downloadStorage', href: '/settings/downloads', anchor: 'storage', keywords: 'download disk space usage cleanup partial', admin: true },
	{ id: 'downloadLimits', href: '/settings/downloads', anchor: 'limits', keywords: 'download quota budget limit free space directory concurrency', admin: true },
	{ id: 'offlineStorage', href: '/settings/offline', anchor: 'storage', keywords: 'offline save download cache browser quota storage read' },
	{ id: 'indexers', href: '/settings/indexers', anchor: 'indexers', keywords: 'indexer torznab newznab jackett prowlarr tracker api key', admin: true },
	{ id: 'acquisitionEngine', href: '/settings/acquisition', anchor: 'engine', keywords: 'torrent engine port peers rate limit speed bittorrent', admin: true },
	{ id: 'acquisitionSeeding', href: '/settings/acquisition', anchor: 'seeding', keywords: 'seed ratio seeding time share upload', admin: true },
	{ id: 'acquisitionImport', href: '/settings/acquisition', anchor: 'import', keywords: 'import hardlink copy move rename library', admin: true },
	{ id: 'acquisitionAutomation', href: '/settings/acquisition', anchor: 'automation', keywords: 'automation rss sync monitored search schedule stalled', admin: true },
	{ id: 'acquisitionProfiles', href: '/settings/acquisition', anchor: 'profiles', keywords: 'quality profile resolution cutoff upgrade group subtitle language', admin: true },
	{ id: 'acquisitionSubtitles', href: '/settings/acquisition', anchor: 'subtitles', keywords: 'subtitle subtitles opensubtitles provider api key captions srt', admin: true }
];

/**
 * Every query word must appear in the text (any order). Earlier and
 * word-initial hits rank higher, so "hard" finds "Hardware acceleration"
 * first and never "Change password".
 */
export function fuzzyScore(query: string, text: string): number {
	const words = query.toLowerCase().split(/\s+/).filter(Boolean);
	if (words.length === 0) return 1;
	const t = text.toLowerCase();
	let score = 0;
	for (const w of words) {
		const at = t.indexOf(w);
		if (at < 0) return 0;
		const initial = at === 0 || /[\s(›/-]/.test(t[at - 1]);
		score += 100 - Math.min(at, 90) + (initial ? 50 : 0);
	}
	return score;
}
