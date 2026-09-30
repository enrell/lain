/*
 * Wire types mirrored from the Go server. Every interface names its
 * source contract so a backend change is easy to trace:
 *   internal/contracts/media.go, internal/auth/auth.go,
 *   internal/gateway/server.go.
 *
 * These are the exact JSON shapes the gateway encodes; do not add
 * fields the server does not send.
 */

/** internal/contracts.CatalogItem (media.go). */
export interface CatalogItem {
	id: string;
	library_id: string;
	kind: string;
	title: string;
	season: number;
	episode: number;
	year: number;
	file_path: string;
	size: number;
	confidence: number;
	origin: string;
	provenance: string;
	updated_at: number;
	/** The file vanished from the library root (D-068): the row is kept
	 * and the flag clears on its own when the path comes back. */
	missing?: boolean;
}

/** internal/contracts.CatalogPage (media.go). */
export interface CatalogPage {
	items: CatalogItem[];
	total: number;
	limit: number;
	offset: number;
}

/** internal/contracts.Library (media.go). */
export interface Library {
	id: string;
	name: string;
	type: string;
	path: string;
	source: string;
	created_at: number;
}

/** GET /api/browse: server-side folder picker payload. */
export interface BrowseDir {
	name: string;
	path: string;
}

export interface BrowseResult {
	path: string;
	parent: string;
	detected: boolean;
	dirs: BrowseDir[];
}

/** internal/contracts.Progress (media.go). */
export interface Progress {
	item_id: string;
	user_id: string;
	position_sec: number;
	duration_sec: number;
	completed: boolean;
	updated_at: number;
}

/** internal/contracts.Plan (media.go). */
export interface PlaybackPlan {
	mode: 'direct' | 'transcode' | 'transcode-required' | string;
	asset: string;
	profile?: string;
	session?: string;
	state?: TranscodeState;
	streams?: MediaStream[];
	available: boolean;
	reason?: string;
	/** Why a transcode/remux is needed (D-042). */
	reasons?: string[];
	/** The probed media length in seconds (0/absent = the server did not
	 * probe). An HLS EVENT playlist only lists produced segments, so this
	 * is the only source of the real timeline while ffmpeg still runs. */
	duration_sec?: number;
}

export type TranscodeState = 'idle' | 'queued' | 'running' | 'ready' | 'failed';
export type TranscodeDelivery = 'hls' | 'progressive';
export type SubtitleMode = 'auto' | 'extract' | 'burn' | 'off';

export interface TranscodeSelection {
	delivery?: TranscodeDelivery;
	quality?: string;
	video_codec?: 'h264' | 'hevc' | 'av1' | string;
	audio_codec?: 'aac' | 'ac3' | 'eac3' | string;
	audio_stream?: number;
	subtitle_stream?: number;
	subtitle_mode?: SubtitleMode;
	/** Source position to begin the session at (resume/seek). */
	start_sec?: number;
	/** Client-side permissions, mirroring Jellyfin's PlaybackInfo flags. */
	allow_video_stream_copy?: boolean;
	allow_audio_stream_copy?: boolean;
}

export interface TranscodeStatus {
	session: string;
	state: TranscodeState;
	profile: string;
	delivery?: TranscodeDelivery;
	method?: 'remux' | 'transcode' | string;
	cached?: boolean;
	/** HLS: the playlist is ready and the client can start playing. */
	playable?: boolean;
	/** Preparation fraction (0..1) while queued or running (D-039). */
	progress?: number;
	/** Live pipeline metrics while a job runs. */
	fps?: number;
	output_bitrate_kbps?: number;
	/** Per-stream direct flags: the stream is copied, not re-encoded. */
	video_direct?: boolean;
	audio_direct?: boolean;
	/** Owning account, present only in the admin session list. */
	user_id?: string;
	has_subtitle?: boolean;
	/** Pipeline facts (D-042): encoder actually used, hardware backend,
	 * software-fallback note and why the transcode is needed. */
	encoder?: string;
	hardware?: string;
	fallback?: string;
	reasons?: string[];
	video_codec?: string;
	audio_codec?: string;
	width?: number;
	height?: number;
	bitrate_kbps?: number;
	error_code?: string;
	error?: string;
	queued_at?: number;
	started_at?: number;
	finished_at?: number;
}

/** internal/contracts.TranscodeQuality (transcode.go). */
export interface TranscodeQuality {
	name: string;
	max_width?: number;
	max_height?: number;
	bitrate_kbps?: number;
}

/** GET /api/playback/options. */
export interface PlaybackOptions {
	default_delivery: TranscodeDelivery;
	deliveries: string[];
	qualities: TranscodeQuality[];
}

/** internal/contracts.TranscodeSettings (transcode.go). */
export interface TranscodeSettings {
	default_delivery: TranscodeDelivery;
	hls_segment_seconds: number;
	hls_segment_container: string;
	throttle: boolean;
	throttle_ahead_sec: number;
	segment_deletion: boolean;
	segment_keep_sec: number;
	idle_timeout_sec: number;
	cache_bytes: number;
	queue_size: number;
	max_concurrent: number;
	thread_count: number;
	max_muxing_queue_size: number;
	transcode_temp_path: string;
	remote_bitrate_limit_kbps: number;
	encoder_preset: string;
	crf: number;
	h264_preset: string;
	h265_preset: string;
	av1_preset: string;
	h264_crf: number;
	h265_crf: number;
	av1_crf: number;
	allow_hevc: boolean;
	allow_av1: boolean;
	qualities: TranscodeQuality[];
	audio_bitrate_kbps: number;
	audio_vbr: boolean;
	downmix_audio: boolean;
	downmix_audio_boost: number;
	downmix_stereo_algorithm: string;
	subtitle_mode: SubtitleMode;
	allow_subtitle_extraction?: boolean;
	fallback_font_path: string;
	fallback_font_enabled?: boolean;
	fallback_font_name: string;
	tone_mapping: boolean;
	tone_mapping_algorithm: string;
	tone_mapping_mode: string;
	tone_mapping_peak_nits: number;
	deinterlace: 'auto' | 'off' | string;
	deinterlace_method: string;
	deinterlace_double_rate: boolean;
	hardware_acceleration: string;
	hardware_decode_codecs: string[];
	hardware_encode: boolean;
	hardware_low_power: boolean;
	hardware_decode_10bit_hevc: boolean;
	hardware_decode_10bit_vp9: boolean;
	hardware_device: string;
	ffmpeg_path: string;
	ffprobe_path: string;
}

/** transcode.Transcoder.Probe (v3.go). */
export interface TranscodeCapabilities {
	ffmpeg: string;
	encoders: string[];
	tone_mapping: boolean;
	tone_mapping_bt2390: boolean;
	hardware: Record<string, boolean>;
}

export interface TranscodeSettingsResponse {
	settings: TranscodeSettings;
	capabilities: TranscodeCapabilities;
}

export interface TranscodeSessionsResponse {
	sessions: TranscodeStatus[];
}

export interface MediaStream {
	index: number;
	type: 'video' | 'audio' | 'subtitle' | string;
	codec: string;
	profile?: string;
	pixel_format?: string;
	width?: number;
	height?: number;
	channels?: number;
	language?: string;
	title?: string;
	default?: boolean;
	forced?: boolean;
	color_transfer?: string;
	color_primaries?: string;
	convertible?: boolean;
}

/** internal/contracts.ScanStats (media.go). */
export interface ScanStats {
	libraries: number;
	candidates: number;
	identified: number;
	unidentified: number;
	errors: number;
	pruned: number;
	/** Files gone from a clean scan (marked, not deleted — D-068). */
	missing: number;
	/** Missing items whose file reappeared under the same path. */
	restored: number;
	migrated: number;
	enriched: number;
	walk_errors: number;
	/** Roots behind walk_errors and any inaccessible root, in library order. */
	unreadable?: { library_id: string; name: string; path: string; reason: string }[];
	dirs: number;
	started_at: number;
	finished_at: number;
}

/** gateway.ScanStatus (server.go). */
export type ScanState = 'idle' | 'running' | 'done' | 'error';

export interface ScanStatus {
	state: ScanState;
	started_at?: number;
	finished_at?: number;
	stats?: ScanStats;
	error?: string;
	/** What started this scan: a button press or the filesystem watcher. */
	trigger?: 'manual' | 'watch';
}

/** internal/contracts.Enrichment (media.go). */
export interface Enrichment {
	item_id: string;
	provider: string;
	remote_id: string;
	title: string;
	year?: number;
	genres?: string[];
	synopsis?: string;
	poster?: string;
	cover?: string;
	fetched_at: number;
}

/** internal/auth.User (auth.go); PassHash is nulled by Public(). */
export interface User {
	id: string;
	/** ISO 639-2 code used to prefer audio, then subtitles (D-071). */
	preferred_language?: string;
	username: string;
	role: 'admin' | 'user';
	disabled: boolean;
	pwd_ver: number;
	created_at: number;
	/** Per-user playback limits (D-042); absent means unrestricted. */
	playback?: UserPlaybackPolicy;
	/** Public face of the account (D-086). */
	profile?: UserProfile;
}

/** internal/auth.Profile (profile.go). */
export interface UserProfile {
	display_name?: string;
	bio?: string;
	avatar: {
		/** '' = initials, 'mascot' = a pre-made mascot, 'upload' = the user's picture. */
		kind?: '' | 'mascot' | 'upload';
		mascot?: string;
		/** Changes with every new picture; part of the image URL for caching. */
		version?: number;
	};
}

/** internal/auth.PlaybackPolicy (auth.go). */
export interface UserPlaybackPolicy {
	allow_video_transcode?: boolean;
	allow_audio_transcode?: boolean;
	allow_remux?: boolean;
	max_bitrate_kbps?: number;
	max_streams?: number;
	subtitle_mode?: SubtitleMode;
}

/** gateway handleSetupStatus. */
export interface SetupStatus {
	setup_required: boolean;
}

/** gateway handleLogin. */
export interface LoginResponse {
	token: string;
}

/** core.BindingView (composition.go) as returned by /api/plugins. */
export interface BindingView {
	capability: string;
	mode: 'exactly-one' | 'ordered-many' | 'first-accepted' | 'merge-many' | 'fan-out' | string;
	providers: string[];
	generation: number;
}

/** core.Event (registry.go). */
export interface RegistryEvent {
	at: number;
	kind: string;
	capability?: string;
	provider?: string;
	generation?: number;
	detail?: string;
}

/** core.ProviderInfo (registry.go). */
export interface ProviderInfo {
	id: string;
	capabilities: string[];
	healthy: boolean;
}

/** gateway handlePlugins. */
export interface PluginsInfo {
	composition: BindingView[];
	providers: string[];
	provider_info: ProviderInfo[];
	events: RegistryEvent[];
}

/** gateway handleSwap error body (409 stale-generation, 422 unhealthy). */
export interface SwapResult {
	generation: number;
	composition: BindingView[];
}

export interface SwapErrorBody {
	error: string;
	code?: string;
	generation?: number;
}

export interface EnrichmentBatch {
	items: Enrichment[];
}
/** gateway linkView (list.go): a linked external list account, token-free. */
export interface LinkedAccountView {
	platform: string;
	remote_user_id: string;
	remote_username: string;
	linked_at: number;
	last_sync_at?: number;
	last_sync_error?: string;
	entry_count: number;
	token_expires_at?: number;
	token_expired?: boolean;
	/** Push finished episodes back to the platform (opt-in). */
	scrobble: boolean;
}

/** GET /api/me/links. */
export interface LinksResponse {
	links: LinkedAccountView[];
}

/** GET /api/me/links/{platform}/authorize. */
export interface AuthorizeResponse {
	url: string;
}

/** POST /api/me/links/{platform}/sync. */
export interface ListSyncStats {
	upserted: number;
	removed: number;
}

export interface SyncResponse {
	status: string;
	stats: ListSyncStats;
}

/** internal/contracts.ListEntry (list.go): one tracked media row. */
export interface ListEntry {
	id: string;
	user_id: string;
	platform: string;
	remote_id: string;
	media_type: string;
	format?: string;
	title: string;
	cover?: string;
	status: string;
	progress: number;
	progress_total?: number;
	progress_volumes?: number;
	score?: number;
	repeat?: number;
	notes?: string;
	started_at?: string;
	completed_at?: string;
	updated_at: number;
}

/** GET /api/list. */
export interface ListResponse {
	entries: ListEntry[];
}

/** GET /api/admin/settings/integrations: per-platform OAuth state. */
export interface IntegrationPlatform {
	client_id: string;
	secret_set: boolean;
	callback_url: string;
}

export interface IntegrationsResponse {
	platforms: Record<string, IntegrationPlatform>;
}

/** Public semantic palette resolved from the server host's Omarchy theme. */
export interface ThemePalette {
	source: 'omarchy' | 'default';
	mode: 'dark' | 'light';
	background: string;
	surface: string;
	surface_hover: string;
	surface_active: string;
	foreground: string;
	muted: string;
	line: string;
	accent: string;
	accent_hover: string;
	accent_foreground: string;
	danger: string;
	danger_foreground: string;
	success: string;
	warning: string;
}

/** One page of a comic or manga archive (internal/contracts/comic.go). */
export interface ReaderPageInfo {
	index: number;
	mime: string;
	size: number;
	/** 0 when the server could not read the image header. */
	width: number;
	height: number;
}

/** GET /api/items/{id}/pages: the page index plus the kind's default direction. */
export interface ReaderView {
	kind: 'comic' | 'manga';
	format: string;
	direction: 'rtl' | 'ltr';
	pages: ReaderPageInfo[];
}
