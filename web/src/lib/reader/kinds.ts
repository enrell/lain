/** Catalog kinds that open in the reader instead of the video player (D-085). */
const READABLE = new Set(['comic', 'manga']);

export function isReadable(kind: string | undefined | null): boolean {
	return !!kind && READABLE.has(kind);
}

/**
 * How a readable file is named in the UI. The catalog stores a volume as
 * the season and a chapter or issue as the episode, each 0 when the file
 * does not carry it, so a volume file, a chapter file and "Vol 2 Ch 15"
 * stay distinguishable.
 */
export function readingLabel(item: {
	kind: string;
	season: number;
	episode: number;
	title: string;
}): string {
	const unit = item.kind === 'manga' ? 'Ch' : 'Issue';
	if (item.season > 0 && item.episode > 0) return `Vol ${item.season} · ${unit} ${item.episode}`;
	if (item.season > 0) return `Vol ${item.season}`;
	if (item.episode > 0) return `${unit} ${item.episode}`;
	return item.title;
}
