/*
 * The single place HTTP happens. Domain modules in this folder call
 * request(); components never build fetch calls by hand.
 *
 * Error policy: every failure becomes an ApiError with a stable kind
 * (unauthorized, forbidden, not-found, validation, conflict,
 * unavailable, server, network) so UI can differentiate without
 * parsing strings. 401 responses notify the session once, which clears
 * local auth and lets the layout redirect to /login.
 */

export type ApiErrorKind =
	| 'unauthorized'
	| 'forbidden'
	| 'not-found'
	| 'validation'
	| 'conflict'
	| 'unavailable'
	| 'server'
	| 'network'
	| 'unknown';

export class ApiError extends Error {
	readonly kind: ApiErrorKind;
	readonly status: number;
	readonly code?: string;
	readonly detail?: unknown;

	constructor(kind: ApiErrorKind, message: string, status = 0, code?: string, detail?: unknown) {
		super(message);
		this.name = 'ApiError';
		this.kind = kind;
		this.status = status;
		this.code = code;
		this.detail = detail;
	}

	get isAuth(): boolean {
		return this.kind === 'unauthorized';
	}
}

type QueryValue = string | number | boolean | undefined | null;

export interface RequestOptions {
	method?: 'GET' | 'POST' | 'PUT' | 'PATCH' | 'DELETE';
	body?: unknown;
	query?: Record<string, QueryValue>;
	signal?: AbortSignal;
	keepalive?: boolean;
	/** Internal: keep 401 handling out of background probes. */
	suppressAuthRedirect?: boolean;
}

let getToken: () => string | null = () => null;
let notifyUnauthorized: (() => void) | null = null;

/** Wired once by the session store; keeps this module free of imports. */
export function configureApi(opts: {
	getToken: () => string | null;
	onUnauthorized?: () => void;
}): void {
	getToken = opts.getToken;
	notifyUnauthorized = opts.onUnauthorized ?? null;
}

export function buildUrl(path: string, query?: Record<string, QueryValue>): string {
	if (!query) return path;
	const qs = new URLSearchParams();
	for (const [key, value] of Object.entries(query)) {
		if (value === undefined || value === null || value === '') continue;
		qs.set(key, String(value));
	}
	const s = qs.toString();
	return s ? `${path}?${s}` : path;
}

function kindForStatus(status: number): ApiErrorKind {
	switch (status) {
		case 400:
			return 'validation';
		case 401:
			return 'unauthorized';
		case 403:
			return 'forbidden';
		case 404:
			return 'not-found';
		case 409:
			return 'conflict';
		case 422:
		case 503:
			return 'unavailable';
		default:
			return status >= 500 ? 'server' : 'unknown';
	}
}

interface ErrorShape {
	error?: unknown;
	code?: unknown;
}

function extractError(parsed: unknown): { message: string; code?: string } {
	if (parsed && typeof parsed === 'object') {
		const body = parsed as ErrorShape;
		const code = typeof body.code === 'string' ? body.code : undefined;
		if (typeof body.error === 'string' && body.error.trim() !== '') {
			return { message: body.error, code };
		}
		if (body.error && typeof body.error === 'object') {
			const nested = body.error as { message?: unknown };
			if (typeof nested.message === 'string') return { message: nested.message, code };
		}
	}
	return { message: '', code: undefined };
}

/*
 * fetchRaw performs the authenticated fetch and maps failures to
 * ApiError, but returns the live Response for callers that need bytes
 * (backup download). Most callers want request() instead.
 */
export async function fetchRaw(path: string, opts: RequestOptions = {}): Promise<Response> {
	const method = opts.method ?? 'GET';
	const headers = new Headers();
	const token = getToken();
	if (token) headers.set('Authorization', `Bearer ${token}`);

	let body: string | undefined;
	if (opts.body !== undefined) {
		headers.set('Content-Type', 'application/json');
		body = JSON.stringify(opts.body);
	}

	let res: Response;
	try {
		res = await fetch(buildUrl(path, opts.query), {
			method,
			headers,
			body,
			signal: opts.signal,
			keepalive: opts.keepalive,
			credentials: 'same-origin'
		});
	} catch (err) {
		if (err instanceof DOMException && err.name === 'AbortError') throw err;
		throw new ApiError('network', 'Cannot reach the Lain server.', 0, undefined, err);
	}

	if (!res.ok) {
		const text = await res.text();
		let parsed: unknown;
		if (text !== '') {
			try {
				parsed = JSON.parse(text);
			} catch {
				parsed = text;
			}
		}
		const { message, code } = extractError(parsed);
		const err = new ApiError(
			kindForStatus(res.status),
			message || `Request failed with status ${res.status}.`,
			res.status,
			code,
			parsed
		);
		if (err.kind === 'unauthorized' && !opts.suppressAuthRedirect) notifyUnauthorized?.();
		if (import.meta.env.DEV) {
			console.error(`[lain-api] ${method} ${path} -> ${res.status}`, parsed);
		}
		throw err;
	}
	return res;
}

export async function request<T>(path: string, opts: RequestOptions = {}): Promise<T> {
	const method = opts.method ?? 'GET';
	const res = await fetchRaw(path, opts);
	if (res.status === 204) return undefined as T;

	const text = await res.text();
	let parsed: unknown;
	if (text !== '') {
		try {
			parsed = JSON.parse(text);
		} catch {
			parsed = text;
		}
	}
	if (import.meta.env.DEV && res.status >= 500) {
		console.error(`[lain-api] ${method} ${path} -> ${res.status}`, parsed);
	}
	return parsed as T;
}

/*
 * Media URLs carry the token as a query parameter because native media
 * elements cannot attach Authorization headers. This helper exists so
 * the token is appended in exactly one place, URL-encoded, and only
 * for media endpoints. Never use it for ordinary API calls.
 */
export function mediaUrl(
	path: string,
	token: string | null,
	params: Record<string, string> = {}
): string {
	const qs = new URLSearchParams(params);
	if (token) qs.set('token', token);
	const s = qs.toString();
	return s ? `${path}?${s}` : path;
}
