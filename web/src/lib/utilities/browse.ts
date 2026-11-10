import type { Enrichment } from '$lib/api/types';
import type { SeriesGroup } from './grouping';

type Lookup = (id: string) => Enrichment | null | undefined;

/** The file whose picture stands for a group on its card. */
export function artItem(group: SeriesGroup) {
	return group.items.find((i) => i.episode > 0) ?? group.items[0];
}

/** Every genre of a group's files, first spelling kept, case ignored. */
export function groupGenres(group: SeriesGroup, enrichment: Lookup): string[] {
	const seen = new Map<string, string>();
	for (const item of group.items) {
		for (const genre of enrichment(item.id)?.genres ?? []) {
			const key = genre.trim().toLowerCase();
			if (key && !seen.has(key)) seen.set(key, genre.trim());
		}
	}
	return [...seen.values()];
}

/** Genres by how many groups carry them, most common first. */
export function genreCounts(groups: SeriesGroup[], enrichment: Lookup): { genre: string; count: number }[] {
	const counts = new Map<string, { genre: string; count: number }>();
	for (const group of groups) {
		for (const genre of groupGenres(group, enrichment)) {
			const key = genre.toLowerCase();
			const entry = counts.get(key);
			if (entry) entry.count += 1;
			else counts.set(key, { genre, count: 1 });
		}
	}
	return [...counts.values()].sort((a, b) => b.count - a.count || a.genre.localeCompare(b.genre));
}

/** Groups that carry every picked genre (all of them when none is picked). */
export function withGenres(groups: SeriesGroup[], picked: string[], enrichment: Lookup): SeriesGroup[] {
	if (picked.length === 0) return groups;
	const wanted = picked.map((g) => g.toLowerCase());
	return groups.filter((group) => {
		const have = new Set(groupGenres(group, enrichment).map((g) => g.toLowerCase()));
		return wanted.every((g) => have.has(g));
	});
}
