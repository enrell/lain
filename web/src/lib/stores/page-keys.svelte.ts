/*
 * What `?` lists for the screen in view. A page or component names the
 * keys it handles where it handles them, `usePageKeys(() => [...])` in its
 * script, and the shell's dialog shows them above the keys that work
 * everywhere. The rows are a function of state, so a key that only works
 * in one tab, for an admin or while something is selected is listed only
 * then; a component that unmounts takes its rows with it.
 */

import { untrack } from 'svelte';
import { SvelteSet } from 'svelte/reactivity';

export interface KeyHint {
	/** Key caps as they are pressed together: ['Ctrl', 'S'], or one cap per alternative label like ['j', 'k']. */
	keys: string[];
	/** What the key does here, already translated. */
	label: string;
}

/** One owner's rows: their own state, so updating them never touches the list of owners. */
class Group {
	rows = $state<KeyHint[]>([]);
}

class PageKeys {
	/*
	 * The owners on screen, in the order they mounted. A set changed in
	 * place, not an array state replaced by a filtered copy: an owner leaves
	 * in its effect's teardown, where Svelte reads state as it was before the
	 * update that is unmounting it, so the copy left out an owner that had
	 * mounted in that same update (a {#key} block re-creating the library
	 * grid lost its rows, and the header's `/` came back over its filter).
	 */
	#groups = new SvelteSet<Group>();

	/** Every row on screen, in the order their owners mounted; a key listed twice keeps its first meaning. */
	get rows(): KeyHint[] {
		const seen = new Set<string>();
		const out: KeyHint[] = [];
		for (const group of this.#groups) {
			for (const row of group.rows) {
				const id = row.keys.join('+');
				if (seen.has(id)) continue;
				seen.add(id);
				out.push(row);
			}
		}
		return out;
	}

	/** Whether the page claims a key of its own (the header's `/` steps aside for a page filter). */
	has(key: string): boolean {
		for (const group of this.#groups) {
			if (group.rows.some((r) => r.keys.length === 1 && r.keys[0] === key)) return true;
		}
		return false;
	}

	// Called while components mount and unmount: whatever is rendering must
	// not come to depend on the set.
	open(): Group {
		const group = new Group();
		untrack(() => this.#groups.add(group));
		return group;
	}

	close(group: Group): void {
		untrack(() => this.#groups.delete(group));
	}
}

export const pageKeys = new PageKeys();

/** Lists the caller's keys in `?` while it is mounted. Call during component initialisation. */
export function usePageKeys(rows: () => KeyHint[]): void {
	const group = pageKeys.open();
	$effect(() => {
		group.rows = rows();
	});
	$effect(() => () => pageKeys.close(group));
}
