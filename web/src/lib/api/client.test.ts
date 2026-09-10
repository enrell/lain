import { beforeEach, describe, expect, it, vi } from 'vitest';
import { ApiError, buildUrl, configureApi, fetchRaw, request } from './client';

function jsonResponse(body: unknown, status = 200): Response {
	return new Response(JSON.stringify(body), {
		status,
		headers: { 'Content-Type': 'application/json' }
	});
}

let unauthorized = 0;

beforeEach(() => {
	unauthorized = 0;
	configureApi({
		getToken: () => null,
		onUnauthorized: () => {
			unauthorized++;
		}
	});
});

describe('buildUrl', () => {
	it('skips empty values and keeps numbers and booleans', () => {
		expect(buildUrl('/api/catalog', { limit: 50, offset: 0, sort: '', q: undefined })).toBe(
			'/api/catalog?limit=50&offset=0'
		);
		expect(buildUrl('/api/search', { q: 'frieren', kind: 'anime' })).toBe(
			'/api/search?q=frieren&kind=anime'
		);
		expect(buildUrl('/health')).toBe('/health');
	});
});

describe('request', () => {
	it('parses JSON success bodies', async () => {
		vi.stubGlobal('fetch', vi.fn(async () => jsonResponse({ status: 'ok' })));
		await expect(request('/api/health')).resolves.toEqual({ status: 'ok' });
	});

	it('attaches the configured bearer token', async () => {
		const fetchMock = vi.fn(async (_url: string, _init?: RequestInit) => jsonResponse({}));
		vi.stubGlobal('fetch', fetchMock);
		configureApi({ getToken: () => 'tok-123' });
		await request('/api/me');
		const init = fetchMock.mock.calls[0][1] as RequestInit;
		const headers = init.headers as Headers;
		expect(headers.get('Authorization')).toBe('Bearer tok-123');
	});

	it('maps validation errors with the server message', async () => {
		vi.stubGlobal('fetch', vi.fn(async () => jsonResponse({ error: 'name and path required' }, 400)));
		const err = await request('/api/libraries', { method: 'POST', body: {} }).catch((e) => e);
		expect(err).toBeInstanceOf(ApiError);
		expect((err as ApiError).kind).toBe('validation');
		expect((err as ApiError).message).toBe('name and path required');
	});

	it('extracts the stable swap error code', async () => {
		vi.stubGlobal(
			'fetch',
			vi.fn(async () =>
				jsonResponse({ error: 'expected generation 1, active is 2', code: 'stale-generation' }, 409)
			)
		);
		const err = (await request('/api/plugins/swap', { method: 'POST', body: {} }).catch((e) => e)) as ApiError;
		expect(err.kind).toBe('conflict');
		expect(err.code).toBe('stale-generation');
	});

	it('treats 404 as not-found and 503 as unavailable', async () => {
		vi.stubGlobal('fetch', vi.fn(async () => jsonResponse({ error: 'unknown item' }, 404)));
		const notFound = (await request('/api/catalog/x').catch((e) => e)) as ApiError;
		expect(notFound.kind).toBe('not-found');

		vi.stubGlobal('fetch', vi.fn(async () => jsonResponse({ error: 'no healthy provider' }, 503)));
		const down = (await request('/api/search').catch((e) => e)) as ApiError;
		expect(down.kind).toBe('unavailable');
	});

	it('notifies the session on 401 and can be suppressed', async () => {
		vi.stubGlobal('fetch', vi.fn(async () => jsonResponse({ error: 'unauthorized' }, 401)));
		await request('/api/me').catch(() => {});
		expect(unauthorized).toBe(1);
		await request('/api/me', { suppressAuthRedirect: true }).catch(() => {});
		expect(unauthorized).toBe(1);
	});

	it('classifies a fetch rejection as a network error', async () => {
		vi.stubGlobal(
			'fetch',
			vi.fn(async () => {
				throw new TypeError('fetch failed');
			})
		);
		const err = (await request('/api/me').catch((e) => e)) as ApiError;
		expect(err.kind).toBe('network');
	});
});

describe('fetchRaw', () => {
	it('returns the live response for byte downloads', async () => {
		vi.stubGlobal(
			'fetch',
			vi.fn(
				async () =>
					new Response(new Uint8Array([1, 2, 3]), {
						status: 200,
						headers: { 'Content-Disposition': 'attachment; filename="lain.db"' }
					})
			)
		);
		const res = await fetchRaw('/api/admin/backup');
		expect(res.headers.get('Content-Disposition')).toContain('lain.db');
		await expect(res.arrayBuffer()).resolves.toBeTruthy();
	});
});
