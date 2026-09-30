/*
 * One source for settings navigation (the rail, the `g`+letter chords)
 * and the command palette index. A setting that exists in the UI but not
 * here is unfindable from Ctrl+K, so rows and entries move together.
 */

export type SettingsScope = 'you' | 'server';

export interface SettingsSection {
	href: string;
	label: string;
	scope: SettingsScope;
	/** Second key of the `g` chord: `g p` opens Profile. Stable across roles. */
	key: string;
	hint: string;
}

export const SECTIONS: SettingsSection[] = [
	{ href: '/settings/profile', label: 'Profile', scope: 'you', key: 'p', hint: 'Name, bio, avatar, account' },
	{ href: '/settings/playback', label: 'Playback', scope: 'you', key: 'y', hint: 'Player, language, video effects' },
	{ href: '/settings/connections', label: 'Connections', scope: 'you', key: 'c', hint: 'AniList and other lists' },
	{ href: '/settings/security', label: 'Security', scope: 'you', key: 's', hint: 'Password and sessions' },
	{ href: '/settings/libraries', label: 'Libraries', scope: 'server', key: 'l', hint: 'Media folders and scans' },
	{ href: '/settings/users', label: 'Users', scope: 'server', key: 'u', hint: 'Accounts and roles' },
	{ href: '/settings/transcoding', label: 'Transcoding', scope: 'server', key: 't', hint: 'How files become streams' },
	{ href: '/settings/integrations', label: 'Integrations', scope: 'server', key: 'i', hint: 'OAuth apps for list sync' },
	{ href: '/settings/plugins', label: 'Plugins', scope: 'server', key: 'x', hint: 'Which provider does each job' },
	{ href: '/settings/backup', label: 'Backup', scope: 'server', key: 'b', hint: 'Database snapshot' }
];

export function visibleSections(admin: boolean): SettingsSection[] {
	return SECTIONS.filter((s) => admin || s.scope === 'you');
}

export function sectionFor(pathname: string): SettingsSection | undefined {
	return SECTIONS.find((s) => pathname === s.href || pathname.startsWith(s.href + '/'));
}

/** A single setting the palette can jump to. `anchor` is a row id on the page. */
export interface SettingEntry {
	label: string;
	href: string;
	anchor: string;
	keywords: string;
	admin?: boolean;
}

export const SETTING_ENTRIES: SettingEntry[] = [
	{ label: 'Display name', href: '/settings/profile', anchor: 'display-name', keywords: 'nickname name profile' },
	{ label: 'Bio', href: '/settings/profile', anchor: 'bio', keywords: 'about description profile' },
	{ label: 'Avatar', href: '/settings/profile', anchor: 'avatar', keywords: 'picture photo mascot icon profile' },
	{ label: 'Sign out', href: '/settings/profile', anchor: 'account', keywords: 'logout log out session' },
	{ label: 'Default player', href: '/settings/playback', anchor: 'player', keywords: 'mpv vlc browser external play' },
	{ label: 'Audio & subtitle language', href: '/settings/playback', anchor: 'language', keywords: 'idioma audio subtitle captions language dub' },
	{ label: 'Video effects', href: '/settings/playback', anchor: 'effects', keywords: 'anime4k upscale shader webgpu effect' },
	{ label: 'AniList', href: '/settings/connections', anchor: 'anilist', keywords: 'anilist list sync scrobble tracker connect' },
	{ label: 'Change password', href: '/settings/security', anchor: 'password', keywords: 'password security credentials' },
	{ label: 'Media libraries', href: '/settings/libraries', anchor: 'libraries', keywords: 'folder path library add remove root', admin: true },
	{ label: 'Library scan', href: '/settings/libraries', anchor: 'scan', keywords: 'scan rescan index refresh', admin: true },
	{ label: 'Users', href: '/settings/users', anchor: 'users', keywords: 'accounts roles admin disable reset password', admin: true },
	{ label: 'Transcoding mode', href: '/settings/transcoding', anchor: 'mode', keywords: 'auto custom policy transcode', admin: true },
	{ label: 'Delivery (HLS, segments)', href: '/settings/transcoding', anchor: 'delivery', keywords: 'hls segment timeout throttle progressive delivery stream', admin: true },
	{ label: 'Encoding & quality ladder', href: '/settings/transcoding', anchor: 'encoding', keywords: 'crf preset hevc av1 h264 quality ladder bitrate deinterlace', admin: true },
	{ label: 'Hardware acceleration', href: '/settings/transcoding', anchor: 'hardware', keywords: 'gpu vaapi nvenc qsv hardware decode encode', admin: true },
	{ label: 'HDR & tone mapping', href: '/settings/transcoding', anchor: 'processing', keywords: 'hdr tone mapping luminance', admin: true },
	{ label: 'Audio & subtitles (server)', href: '/settings/transcoding', anchor: 'audio', keywords: 'audio bitrate downmix subtitle burn font', admin: true },
	{ label: 'Resources & storage', href: '/settings/transcoding', anchor: 'resources', keywords: 'cache threads queue storage path ffmpeg', admin: true },
	{ label: 'Active transcode sessions', href: '/settings/transcoding', anchor: 'sessions', keywords: 'sessions running jobs cancel', admin: true },
	{ label: 'AniList OAuth app', href: '/settings/integrations', anchor: 'integrations', keywords: 'oauth client id secret anilist', admin: true },
	{ label: 'Plugins & capabilities', href: '/settings/plugins', anchor: 'capabilities', keywords: 'plugin provider capability composition replace', admin: true },
	{ label: 'Download backup', href: '/settings/backup', anchor: 'backup', keywords: 'backup snapshot database export', admin: true }
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
