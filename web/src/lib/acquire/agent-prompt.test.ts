/*
 * The coding-agent prompt (D-290, D-291) carries a live key: it must
 * send it only in a header, never in a URL, and point the agent at the
 * server's guide and the ask-first rule.
 */
import { describe, expect, it } from 'vitest';
import { interpolate } from '$lib/i18n/core';
import en from '$lib/i18n/messages/en';

const KEY = 'lain_0123456789abcdef';
const SERVER = 'http://127.0.0.1:9393';

function prompt(): string {
	return interpolate(
		en.acquire.agent.prompt,
		{ server: SERVER, key: KEY, guide: `${SERVER}/api/agent/indexers.md`, expires: '2026-11-13T12:00:00.000Z' },
		String
	);
}

describe('the coding agent prompt', () => {
	it('fills every placeholder', () => {
		expect(prompt()).not.toMatch(/\{\w+\}/);
	});

	it('names the key once, outside any URL, and says it goes in a header', () => {
		const text = prompt();
		expect(text.split(KEY).length - 1).toBe(1);
		for (const line of text.split('\n').filter((l) => l.includes(KEY))) expect(line).not.toMatch(/https?:\/\/|[?&]token=/);
		expect(text).toContain('Authorization: Bearer <key>');
		expect(text).not.toMatch(/token=/);
	});

	it('points at the guide and every indexer call, and asks first', () => {
		const text = prompt();
		expect(text).toContain(`curl -s ${SERVER}/api/agent/indexers.md`);
		for (const call of ['GET /api/acquire/indexers', 'POST /api/acquire/indexers/discover', 'POST /api/acquire/indexers/import', 'POST /api/acquire/indexers/probe', 'DELETE /api/acquire/indexers/<id>'])
			expect(text).toContain(call);
		expect(text).toMatch(/Ask me before you add, change or remove anything/);
	});
});
