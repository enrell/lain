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
	remove: (id: string, deleteData: boolean) =>
		request<{ removed: boolean }>(`/api/acquire/grabs/${seg(id)}`, {
			method: 'DELETE',
			query: { data: deleteData ? '1' : undefined }
		})
};
