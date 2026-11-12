import { describe, expect, it } from 'vitest';
import { MOBILE_BAR, MOBILE_NAV, visibleNav } from './nav-items';

describe('mobile navigation', () => {
	it('shows Acquire to admins only, before Settings', () => {
		const user = visibleNav(MOBILE_NAV, false).map((e) => e.href);
		const admin = visibleNav(MOBILE_NAV, true).map((e) => e.href);
		expect(user).not.toContain('/acquire');
		expect(admin).toContain('/acquire');
		expect(admin.indexOf('/acquire')).toBe(admin.indexOf('/settings') - 1);
		expect(admin.at(-1)).toBe('/settings');
	});
	it('shows Social to everyone', () => {
		expect(visibleNav(MOBILE_NAV, false).map((e) => e.href)).toContain('/social');
		expect(visibleNav(MOBILE_NAV, true).map((e) => e.href)).toContain('/social');
	});
	it('keeps Search in the bar: the phone has no header field', () => {
		expect(MOBILE_BAR).toContain('/search');
		for (const href of MOBILE_BAR) expect(visibleNav(MOBILE_NAV, false).map((e) => e.href)).toContain(href);
	});
	it('keeps every entry unique', () => {
		const hrefs = MOBILE_NAV.map((e) => e.href);
		expect(new Set(hrefs).size).toBe(hrefs.length);
	});
});
