import { describe, expect, it } from 'vitest';
import { embeddedChoice, sidecarIndex, sidecarLabel, sidecarValue, toggledSubtitle, type SidecarTrack } from './sidecars';

const sc = (index: number, language: string, extra: Partial<SidecarTrack> = {}): SidecarTrack => ({
	index, name: `x.${language}.srt`, language, format: 'srt', ...extra
});

describe('sidecar subtitle choices', () => {
	it('keeps sidecar and embedded choices apart', () => {
		expect(sidecarValue(2)).toBe('sc:2');
		expect(sidecarIndex('sc:2')).toBe(2);
		expect(sidecarIndex('3')).toBeNull();
		expect(sidecarIndex('sc:x')).toBeNull();
		expect(embeddedChoice('sc:2')).toBe('');
		expect(embeddedChoice('3')).toBe('3');
		expect(embeddedChoice('')).toBe('');
	});
	it('labels sidecars', () => {
		expect(sidecarLabel(sc(0, 'eng', { tag: 'en' }))).toBe('en · file');
		expect(sidecarLabel(sc(1, 'por', { tag: 'pt-BR', forced: true }))).toBe('pt-BR · file · forced');
		expect(sidecarLabel(sc(2, 'und', { hi: true }))).toBe('x.und.srt · file · SDH');
	});
});

describe('toggledSubtitle (the player’s C key)', () => {
	const choices = ['2', sidecarValue(0), sidecarValue(1)];

	it('turns a track off', () => {
		expect(toggledSubtitle(sidecarValue(1), '', choices)).toBe('');
	});

	it('turns the track that was on back on', () => {
		expect(toggledSubtitle('', sidecarValue(1), choices)).toBe(sidecarValue(1));
	});

	it('falls back to the first track when none was on or it is gone', () => {
		expect(toggledSubtitle('', '', choices)).toBe('2');
		expect(toggledSubtitle('', sidecarValue(7), choices)).toBe('2');
	});

	it('stays off with nothing to show', () => {
		expect(toggledSubtitle('', '2', [])).toBe('');
	});
});
