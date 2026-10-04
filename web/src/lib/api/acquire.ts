import { request } from './client';

/*
 * Acquisition (docs/slices/acquisition.md). Shapes mirror
 * internal/acquire and internal/contracts/{release,indexer}.go. Every
 * route is admin-only. API keys are write-only: the server reports
 * has_api_key and never returns the key.
 */

export interface Release {
	title: string;
	episode_title?: string;
	year?: number;
	season?: number;
	episodes?: number[];
	absolute?: boolean;
	volume?: number;
	chapter?: number;
	season_pack?: boolean;
	group?: string;
	resolution?: string;
	source?: string;
	codec?: string;
	proper?: boolean;
	repack?: boolean;
	version?: number;
	parser: string;
}

export interface IndexerCaps {
	search: boolean;
	tv_search: boolean;
	movie_search: boolean;
	book_search: boolean;
	categories: { id: number; name: string }[];
	limit?: number;
}

export interface Indexer {
	id: string;
	name: string;
	protocol: 'torznab' | 'newznab';
	url: string;
	categories: number[];
	enabled: boolean;
	priority: number;
	min_interval_ms: number;
	caps?: IndexerCaps;
	last_check_at?: number;
	last_error?: string;
	created_at: number;
	has_api_key: boolean;
}

export interface IndexerInput {
	name?: string;
	protocol?: 'torznab' | 'newznab';
	url?: string;
	/** Empty keeps the stored key; "-" clears it. */
	api_key?: string;
	categories?: number[];
	enabled?: boolean;
	priority?: number;
	min_interval_ms?: number;
}

export interface SearchResult {
	indexer_id: string;
	protocol: 'torrent' | 'usenet';
	title: string;
	guid?: string;
	link?: string;
	magnet?: string;
	info_hash?: string;
	size: number;
	seeders: number;
	peers: number;
	published_at?: number;
	categories?: number[];
	comments_url?: string;
	freeleech?: boolean;
}

export interface Candidate extends SearchResult {
	release: Release;
	score: number;
	rejections?: string[];
}

export interface SearchResponse {
	candidates: Candidate[];
	failures: { indexer_id: string; name: string; error: string }[];
}

export type GrabState =
	| 'queued'
	| 'metadata'
	| 'downloading'
	| 'importing'
	| 'seeding'
	| 'done'
	| 'paused'
	| 'failed';

export interface Grab {
	id: string;
	title: string;
	indexer_id?: string;
	source: 'torrent-url' | 'magnet';
	info_hash?: string;
	release: Release;
	library_id: string;
	kind: string;
	state: GrabState;
	size: number;
	downloaded: number;
	uploaded: number;
	completed: number;
	peers: number;
	down_rate: number;
	up_rate: number;
	imported?: string[];
	code?: string;
	error?: string;
	created_at: number;
	updated_at: number;
	finished_at?: number;
	seeding_at?: number;
	data_removed?: boolean;
}

export interface AcquireSettings {
	dir: string;
	listen_port: number;
	max_active: number;
	max_peers: number;
	upload_kbps: number;
	download_kbps: number;
	seed_ratio: number;
	seed_minutes: number;
	remove_after_seeding: boolean;
	import_mode: 'hardlink' | 'copy' | 'move';
	automation: boolean;
	rss_minutes: number;
	search_hours: number;
	stall_hours: number;
	subtitle_hours: number;
}

export interface AcquireUsage {
	used_bytes: number;
	downloads_bytes: number;
	max_bytes: number;
	min_free_bytes: number;
}

export interface AcquireView {
	settings: AcquireSettings;
	usage: AcquireUsage;
	port: number;
	parser: boolean;
}

export interface SearchParams {
	q: string;
	kind?: string;
	season?: number;
	episode?: number;
	year?: number;
}

// ---- Phase 2: automation ----

export type Numbering = 'absolute' | 'seasonal' | 'chapter' | 'volume' | 'movie';

export interface Unit {
	season: number;
	number: number;
}

export interface SeasonStart {
	season: number;
	first: number;
}

export interface SeasonWant {
	season: number;
	from: number;
	to?: number;
}

export interface Monitored {
	id: string;
	library_id: string;
	kind: string;
	title: string;
	aliases: string[];
	year?: number;
	profile_id: string;
	numbering: Numbering;
	season_map?: SeasonStart[];
	from?: number;
	to?: number;
	seasons?: SeasonWant[];
	metadata_episodes?: number;
	enabled: boolean;
	last_search_at?: number;
	created_at: number;
}

export interface Wanted {
	missing: Unit[];
	open_from?: Unit;
	open_seasons?: Record<string, number>;
	upgrades: { unit: Unit; quality: string; path: string }[];
	present: number;
}

export interface WantedRow extends Monitored {
	wanted: Wanted;
	missing_count: number;
	upgrade_count: number;
}

export interface AutomationState {
	enabled: boolean;
	paused?: string;
	last_rss_at?: number;
}

export interface AutomationReport {
	grabbed: string[];
	considered: number;
	rejected: number;
	paused?: string;
	failures?: { indexer_id: string; name: string; error: string }[];
}

export interface Profile {
	id: string;
	name: string;
	resolutions: string[];
	sources: string[];
	cutoff: string;
	preferred_groups: string[];
	blocked_groups: string[];
	blocked_words: string[];
	min_seeders: number;
	min_size_mb: number;
	max_size_mb: number;
	prefer_proper: boolean;
	subtitle_languages: string[];
	subtitle_even_with_audio: boolean;
	subtitle_hi: 'include' | 'prefer' | 'exclude';
}

/* Phase 3: subtitles (A-28…A-37). */

export interface SubtitleProvider {
	id: string;
	name: string;
	kind: 'opensubtitles';
	base_url?: string;
	username?: string;
	enabled: boolean;
	priority: number;
	has_api_key: boolean;
	has_password: boolean;
	created_at: number;
}

/** Secrets: '' keeps the stored value, '-' clears it. */
export interface SubtitleProviderInput {
	name?: string;
	kind?: 'opensubtitles';
	base_url?: string;
	api_key?: string;
	username?: string;
	password?: string;
	enabled?: boolean;
	priority?: number;
}

export interface SubtitleChoice {
	provider_id: string;
	file_id: string;
	language: string;
	region?: string;
	release?: string;
	file_name?: string;
	hash_match?: boolean;
	hi?: boolean;
	forced?: boolean;
	downloads?: number;
	accepted: boolean;
	rejections?: string[];
	score: number;
}

export interface SidecarInfo {
	name: string;
	language: string;
	tag?: string;
	forced?: boolean;
	hi?: boolean;
	format: string;
}

export interface FileSubtitles {
	item_id: string;
	path: string;
	missing: string[];
	sidecars: SidecarInfo[];
}

export interface SubtitleRecord {
	path: string;
	media_path: string;
	provider_id: string;
	file_id: string;
	language: string;
	hash_match?: boolean;
	at: number;
}

export interface BlockEntry {
	id: string;
	info_hash?: string;
	title: string;
	indexer_id?: string;
	monitored_id?: string;
	reason: string;
	at: number;
}

export interface HeldFile {
	grab_id: string;
	path: string;
	size: number;
	at: number;
}

const seg = encodeURIComponent;

export const acquire = {
	settings: () => request<AcquireView>('/api/acquire/settings'),
	saveSettings: (body: AcquireSettings) =>
		request<AcquireView>('/api/acquire/settings', { method: 'PUT', body }),

	indexers: () => request<{ indexers: Indexer[] }>('/api/acquire/indexers'),
	createIndexer: (body: IndexerInput) =>
		request<Indexer>('/api/acquire/indexers', { method: 'POST', body }),
	updateIndexer: (id: string, body: IndexerInput) =>
		request<Indexer>(`/api/acquire/indexers/${seg(id)}`, { method: 'PATCH', body }),
	deleteIndexer: (id: string) =>
		request<{ removed: boolean }>(`/api/acquire/indexers/${seg(id)}`, { method: 'DELETE' }),
	testIndexer: (id: string) =>
		request<{ indexer: Indexer; ok: boolean; error?: string }>(`/api/acquire/indexers/${seg(id)}/test`, {
			method: 'POST'
		}),

	search: (p: SearchParams) =>
		request<SearchResponse>('/api/acquire/search', {
			query: { q: p.q, kind: p.kind, season: p.season || undefined, episode: p.episode || undefined, year: p.year || undefined }
		}),
	parse: (name: string, kind?: string) => request<Release>('/api/acquire/parse', { query: { name, kind } }),

	grabs: () => request<{ grabs: Grab[]; usage: AcquireUsage }>('/api/acquire/grabs'),
	grab: (body: { result?: SearchResult; url?: string; magnet?: string; title?: string; library_id: string }) =>
		request<Grab>('/api/acquire/grabs', { method: 'POST', body }),
	action: (id: string, action: 'pause' | 'resume' | 'import') =>
		request<Grab>(`/api/acquire/grabs/${seg(id)}/${action}`, { method: 'POST' }),
	profiles: () => request<{ profiles: Profile[] }>('/api/acquire/profiles'),
	createProfile: (body: Omit<Profile, 'id'>) => request<Profile>('/api/acquire/profiles', { method: 'POST', body }),
	updateProfile: (id: string, body: Omit<Profile, 'id'>) =>
		request<Profile>(`/api/acquire/profiles/${seg(id)}`, { method: 'PUT', body }),
	deleteProfile: (id: string) => request<{ removed: boolean }>(`/api/acquire/profiles/${seg(id)}`, { method: 'DELETE' }),

	wanted: () => request<{ titles: WantedRow[]; automation: AutomationState }>('/api/acquire/wanted'),
	monitor: (body: Partial<Monitored>) => request<Monitored>('/api/acquire/monitored', { method: 'POST', body }),
	updateMonitored: (id: string, body: Partial<Monitored>) =>
		request<Monitored>(`/api/acquire/monitored/${seg(id)}`, { method: 'PUT', body }),
	unmonitor: (id: string) => request<{ removed: boolean }>(`/api/acquire/monitored/${seg(id)}`, { method: 'DELETE' }),
	searchMonitored: (id: string) =>
		request<AutomationReport>(`/api/acquire/monitored/${seg(id)}/search`, { method: 'POST' }),
	rss: () => request<AutomationReport>('/api/acquire/rss', { method: 'POST' }),

	blocklist: () => request<{ blocklist: BlockEntry[] }>('/api/acquire/blocklist'),
	unblock: (id: string) => request<{ removed: boolean }>(`/api/acquire/blocklist/${seg(id)}`, { method: 'DELETE' }),
	replaced: () => request<{ files: HeldFile[] }>('/api/acquire/replaced'),
	purgeReplaced: () => request<{ deleted: number }>('/api/acquire/replaced/purge', { method: 'POST' }),

	subtitleProviders: () => request<{ providers: SubtitleProvider[] }>('/api/acquire/subtitle-providers'),
	createSubtitleProvider: (body: SubtitleProviderInput) =>
		request<SubtitleProvider>('/api/acquire/subtitle-providers', { method: 'POST', body }),
	updateSubtitleProvider: (id: string, body: SubtitleProviderInput) =>
		request<SubtitleProvider>(`/api/acquire/subtitle-providers/${seg(id)}`, { method: 'PUT', body }),
	deleteSubtitleProvider: (id: string) =>
		request<{ removed: boolean }>(`/api/acquire/subtitle-providers/${seg(id)}`, { method: 'DELETE' }),
	searchSubtitles: (itemId: string, languages?: string) =>
		request<{ choices: SubtitleChoice[]; languages: string[] }>(`/api/acquire/items/${seg(itemId)}/subtitles`, {
			query: { languages: languages || undefined }
		}),
	downloadSubtitle: (itemId: string, choice: SubtitleChoice, replace = false) =>
		request<SubtitleRecord>(`/api/acquire/items/${seg(itemId)}/subtitles`, {
			method: 'POST',
			body: { ...choice, replace }
		}),
	monitoredSubtitles: (id: string) =>
		request<{ files: FileSubtitles[] }>(`/api/acquire/monitored/${seg(id)}/subtitles`),
	fetchMonitoredSubtitles: (id: string) =>
		request<{ written: number }>(`/api/acquire/monitored/${seg(id)}/subtitles`, { method: 'POST' }),
	subtitleLedger: () => request<{ subtitles: SubtitleRecord[] }>('/api/acquire/subtitles'),

	remove: (id: string, deleteData: boolean) =>
		request<{ removed: boolean }>(`/api/acquire/grabs/${seg(id)}`, {
			method: 'DELETE',
			query: { data: deleteData ? '1' : undefined }
		})
};
