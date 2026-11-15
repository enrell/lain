import { describe, expect, it } from 'vitest';
import { SECTIONS, SETTING_ENTRIES, entryLabel, entryVisible, fuzzyScore, sectionFor, sectionHint, sectionLabel, visibleSections } from './sections';

describe('settings sections', () => {
	it('gives every section a unique chord key', () => {
		const keys = SECTIONS.map((s) => s.key);
		expect(new Set(keys).size).toBe(keys.length);
	});
	it('hides server sections from non-admins', () => {
		expect(visibleSections(false).every((s) => s.scope === 'you')).toBe(true);
		expect(visibleSections(true)).toHaveLength(SECTIONS.length);
	});
	it('shows a manager the libraries page and nothing else of the server', () => {
		const server = visibleSections(false, true)
			.filter((s) => s.scope === 'server')
			.map((s) => s.id);
		expect(server).toEqual(['libraries']);
		const ids = SETTING_ENTRIES.filter((e) => entryVisible(e, false, true)).map((e) => e.id);
		expect(ids).toContain('libraries');
		expect(ids).toContain('scan');
		expect(ids).not.toContain('roots');
		expect(ids).not.toContain('indexers');
		expect(ids).not.toContain('acquisitionSubtitles');
		expect(ids).not.toContain('downloadQueue');
		expect(ids).not.toContain('users');
		expect(ids).not.toContain('plugins');
		expect(ids).not.toContain('backup');
		expect(ids).not.toContain('integrations');
		expect(ids).not.toContain('mode');
	});
	it('resolves nested paths to their section', () => {
		expect(sectionFor('/settings/libraries/x')?.id).toBe('libraries');
		expect(sectionFor('/elsewhere')).toBeUndefined();
	});
	it('points every entry at a known section', () => {
		for (const e of SETTING_ENTRIES) expect(sectionFor(e.href), e.id).toBeDefined();
	});
	it('has a message for every section and entry', () => {
		for (const s of SECTIONS) {
			expect(sectionLabel(s)).not.toContain('settings.');
			expect(sectionHint(s)).not.toContain('settings.');
		}
		for (const e of SETTING_ENTRIES) expect(entryLabel(e)).not.toContain('settings.');
	});
	it('requires every word, in any order, and prefers word starts', () => {
		expect(fuzzyScore('hard', 'Change password')).toBe(0);
		expect(fuzzyScore('accel hard', 'Hardware acceleration')).toBeGreaterThan(0);
		expect(fuzzyScore('ware', 'Hardware acceleration')).toBeLessThan(fuzzyScore('hard', 'Hardware acceleration'));
		expect(fuzzyScore('zzz', 'Hardware acceleration')).toBe(0);
	});
});
