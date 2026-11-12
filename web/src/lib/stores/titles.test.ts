import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { CatalogListOptions } from '$lib/api/catalog';
import type { CatalogItem, CatalogPage } from '$lib/api/types';

/*
 * `listTitles` asks for whole titles (D-283) and falls back to grouping
 * files when the server pages files, so a show's card counts all of its
 * episodes either way.
 */
const state = vi.hoisted(() => ({
	grouped: true,
	asked: [] as CatalogListOptions[],
	episodes: 0
}));

function file(id: string, title: string, episode: number, updated: number): CatalogItem {
	return {
		id, library_id: 'lib', kind: episode > 0 ? 'episode' : 'video', title, season: 0, episode, year: 0,
		file_path: `${id}.mkv`, size: 1, confidence: 1, origin: '', provenance: '', updated_at: updated
	};
}
// Two shows of three episodes and a film, the newest file last.
const all = [
	file('a1', 'Alpha', 1, 1), file('a2', 'Alpha', 2, 2), file('a3', 'Alpha', 3, 3),
	file('f', 'Film', 0, 4),
	file('b1', 'Beta', 1, 5), file('b2', 'Beta', 2, 6), file('b3', 'Beta', 3, 7)
];

vi.mock('$lib/api', () => ({
	api: {
		catalog: {
			list: async (opts: CatalogListOptions): Promise<CatalogPage> => {
				state.asked.push(opts);
				const newest = [...all].sort((x, y) => y.updated_at - x.updated_at);
				if (opts.group === 'title' && state.grouped) {
					// Beta, then Film, then Alpha; each whole.
					const items = [...all.filter((i) => i.title === 'Beta'), all[3], ...all.filter((i) => i.title === 'Alpha')];
					return { items: items.slice(0, opts.limit === 2 ? 4 : 7), total: 3, limit: opts.limit ?? 50, offset: 0, group: 'title' };
				}
				return { items: newest.slice(0, opts.limit), total: all.length, limit: opts.limit ?? 50, offset: 0 };
			},
			episodes: async (id: string) => {
				state.episodes += 1;
				const title = all.find((i) => i.id === id)!.title;
				return { items: all.filter((i) => i.title === title) };
			}
		}
	}
}));

const { listTitles, titlesOf } = await import('./titles');

beforeEach(() => {
	state.asked = [];
	state.episodes = 0;
});

describe('listTitles', () => {
	it('takes whole titles and their count from a server that pages titles', async () => {
		state.grouped = true;
		const page = await listTitles({ sort: 'recent', limit: 2 });
		expect(page.groups.map((g) => [g.title, g.count])).toEqual([['Beta', 3], ['Film', 1]]);
		expect(page.total).toBe(3);
		expect(state.asked).toEqual([{ sort: 'recent', limit: 2, group: 'title' }]);
		expect(state.episodes).toBe(0);
	});

	it('groups an older server\'s files and asks for each show whole', async () => {
		state.grouped = false;
		const page = await listTitles({ sort: 'recent', limit: 2 });
		expect(page.groups.map((g) => [g.title, g.count])).toEqual([['Beta', 3], ['Film', 1]]);
		expect(page.total).toBe(-1);
		// The page of two files named one title: more files were asked for.
		expect(state.asked.map((o) => o.limit)).toEqual([2, 20]);
		// Every file was on that page, so no show had to be asked for again.
		expect(state.episodes).toBe(0);
	});
});

describe('titlesOf', () => {
	it('makes each kept file its whole title, once, and asks only for the ones it keeps', async () => {
		const kept = [all[4], all[5], all[3], all[0]];
		expect((await titlesOf(kept)).map((g) => [g.title, g.count])).toEqual([['Beta', 3], ['Film', 1], ['Alpha', 3]]);
		state.episodes = 0;
		// A rail's worth: the show past it is never asked for.
		expect((await titlesOf(kept, 2)).map((g) => g.title)).toEqual(['Beta', 'Film']);
		expect(state.episodes).toBe(1);
	});
});
