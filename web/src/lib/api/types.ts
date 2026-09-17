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
}

export type TranscodeState = 'idle' | 'queued' | 'running' | 'ready' | 'failed';

export interface TranscodeSelection {
	profile?: string;
	audio_stream?: number;
	subtitle_stream?: number;
}

export interface TranscodeStatus {
	session: string;
	state: TranscodeState;
	profile: string;
	method?: 'remux' | 'transcode' | string;
	cached?: boolean;
	has_subtitle?: boolean;
	error_code?: string;
	error?: string;
	queued_at?: number;
	started_at?: number;
	finished_at?: number;
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
	username: string;
	role: 'admin' | 'user';
	disabled: boolean;
	pwd_ver: number;
	created_at: number;
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
