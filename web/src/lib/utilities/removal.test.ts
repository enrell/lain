import { describe, expect, it } from 'vitest';
import type { ItemRemoval } from '$lib/api/types';
import { removalMessage, totalRemovals } from './removal';

const removal = (over: Partial<ItemRemoval> = {}): ItemRemoval => ({
	file: true,
	freed_bytes: 0,
	sidecars: 0,
	folders: 0,
	downloads: 0,
	not_wanted: false,
	still_linked: false,
	...over
});

describe('removal totals', () => {
	it('adds up what each delete did and skips an older server\'s silence', () => {
		const totals = totalRemovals([
			removal({ freed_bytes: 1024, downloads: 1 }),
			undefined,
			removal({ freed_bytes: 1024, not_wanted: true, still_linked: true })
		]);
		expect(totals).toEqual({ freed: 2048, downloads: 1, notWanted: true, stillLinked: 1 });
	});

	it('says only what happened', () => {
		expect(removalMessage(1, totalRemovals([removal()]))).toBe('Deleted 1 file.');
		expect(removalMessage(3, { freed: 2048, downloads: 1, notWanted: true, stillLinked: 0 })).toBe(
			'Deleted 3 files. Freed 2.0 KiB. Stopped 1 seeding download. Monitoring will not fetch it again.'
		);
		expect(removalMessage(1, { freed: 0, downloads: 0, notWanted: false, stillLinked: 1 })).toContain('another hard link');
	});
});
