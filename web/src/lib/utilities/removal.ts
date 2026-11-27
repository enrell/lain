import type { ItemRemoval } from '$lib/api/types';
import { t } from '$lib/i18n';
import { formatBytes } from '$lib/utilities/format';

/** What a run of deletes did, added up (D-301). */
export interface RemovalTotals {
	freed: number;
	downloads: number;
	notWanted: boolean;
	stillLinked: number;
}

/** Adds up the `removal` of each deleted item; an older server answers none. */
export function totalRemovals(list: (ItemRemoval | undefined)[]): RemovalTotals {
	const out: RemovalTotals = { freed: 0, downloads: 0, notWanted: false, stillLinked: 0 };
	for (const r of list) {
		if (!r) continue;
		out.freed += r.freed_bytes;
		out.downloads += r.downloads;
		out.notWanted ||= r.not_wanted;
		if (r.still_linked) out.stillLinked += 1;
	}
	return out;
}

/** The sentence said after `count` files were deleted: what it freed and what else it changed. */
export function removalMessage(count: number, totals: RemovalTotals): string {
	return [
		t('title.delete.deleted', { count }),
		totals.freed > 0 ? t('title.delete.freed', { size: formatBytes(totals.freed) }) : '',
		totals.downloads > 0 ? t('title.delete.downloads', { count: totals.downloads }) : '',
		totals.notWanted ? t('title.delete.notWanted') : '',
		totals.stillLinked > 0 ? t('title.delete.stillLinked', { count: totals.stillLinked }) : ''
	]
		.filter(Boolean)
		.join(' ');
}
