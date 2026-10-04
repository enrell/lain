import type { AcquireSettings, Grab, GrabState, Release } from '$lib/api';

const pad = (n: number, w = 2) => String(n).padStart(w, '0');

/**
 * The numbering a release covers, in the notation its kind uses:
 * "S01E02", "S01E05–E06", "S02 pack", "05" (absolute), "01–12",
 * "v01 c003". Technical tokens, so they stay untranslated.
 */
export function releaseNumbers(r: Release): string {
	if (r.volume || r.chapter) {
		return [r.volume ? `v${pad(r.volume)}` : '', r.chapter ? `c${pad(r.chapter, 3)}` : ''].filter(Boolean).join(' ');
	}
	const eps = r.episodes ?? [];
	const first = eps[0];
	const last = eps[eps.length - 1];
	if (r.season && !r.absolute) {
		if (eps.length === 0) return `S${pad(r.season)}${r.season_pack ? ' pack' : ''}`;
		return eps.length > 1 ? `S${pad(r.season)}E${pad(first)}–E${pad(last)}` : `S${pad(r.season)}E${pad(first)}`;
	}
	if (eps.length === 0) return '';
	return eps.length > 1 ? `${pad(first)}–${pad(last)}` : pad(first);
}

/** Quality tokens worth showing: resolution, source, codec, flags. */
export function releaseTags(r: Release): string[] {
	const tags = [r.resolution, r.source, r.codec].filter((t): t is string => !!t);
	if (r.proper) tags.push('proper');
	if (r.repack) tags.push('repack');
	if (r.version && r.version > 1) tags.push(`v${r.version}`);
	return tags;
}

/** States that still move bytes or wait to. */
export function isActiveGrab(s: GrabState): boolean {
	return s === 'queued' || s === 'metadata' || s === 'downloading' || s === 'importing';
}

/** 0..1 download progress; done and seeding are complete. */
export function grabProgress(g: Pick<Grab, 'state' | 'completed' | 'size'>): number {
	if (g.state === 'done' || g.state === 'seeding') return 1;
	if (g.size <= 0) return 0;
	return Math.min(1, Math.max(0, g.completed / g.size));
}

/** Upload ratio with two decimals, '—' before any byte is known. */
export function ratio(g: Pick<Grab, 'uploaded' | 'size'>): string {
	if (g.size <= 0) return '—';
	return (g.uploaded / g.size).toFixed(2);
}

/** "5000, 5070 ,x" -> [5000, 5070]; junk is dropped, order kept, no duplicates. */
export function parseCategories(raw: string): number[] {
	const out: number[] = [];
	for (const part of raw.split(/[\s,]+/)) {
		const n = Number.parseInt(part, 10);
		if (Number.isFinite(n) && n > 0 && String(n) === part.trim() && !out.includes(n)) out.push(n);
	}
	return out;
}

/** How many staged fields differ from the saved settings. */
export function settingsChanges(saved: AcquireSettings, draft: AcquireSettings): number {
	return (Object.keys(saved) as (keyof AcquireSettings)[]).filter((k) => String(saved[k]) !== String(draft[k])).length;
}

/** Search kind for a library type (the server maps it to t=tvsearch, …). */
export const LIBRARY_KINDS = ['anime', 'series', 'movie', 'manga', 'comic'] as const;
