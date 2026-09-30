import { describe, expect, it } from 'vitest';
import { SECTIONS, SETTING_ENTRIES, fuzzyScore, sectionFor, visibleSections } from './sections';

describe('settings sections', () => {
	it('gives every section a unique chord key', () => {
		const keys = SECTIONS.map((s) => s.key);
		expect(new Set(keys).size).toBe(keys.length);
	});
	it('hides server sections from non-admins', () => {
		expect(visibleSections(false).every((s) => s.scope === 'you')).toBe(true);
		expect(visibleSections(true)).toHaveLength(SECTIONS.length);
	});
	it('resolves nested paths to their section', () => {
		expect(sectionFor('/settings/libraries/x')?.label).toBe('Libraries');
		expect(sectionFor('/elsewhere')).toBeUndefined();
	});
	it('points every entry at a known section', () => {
		for (const e of SETTING_ENTRIES) expect(sectionFor(e.href), e.label).toBeDefined();
	});
	it('requires every word, in any order, and prefers word starts', () => {
		expect(fuzzyScore('hard', 'Change password')).toBe(0);
		expect(fuzzyScore('accel hard', 'Hardware acceleration')).toBeGreaterThan(0);
		expect(fuzzyScore('ware', 'Hardware acceleration')).toBeLessThan(fuzzyScore('hard', 'Hardware acceleration'));
		expect(fuzzyScore('zzz', 'Hardware acceleration')).toBe(0);
	});
});
