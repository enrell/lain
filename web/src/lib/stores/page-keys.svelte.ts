/*
 * What `?` lists for the screen in view. A page or component names the
 * keys it handles where it handles them, `usePageKeys(() => [...])` in its
 * script, and the shell's dialog shows them above the keys that work
 * everywhere. The rows are a function of state, so a key that only works
 * in one tab, for an admin or while something is selected is listed only
 * then; a component that unmounts takes its rows with it.
 */

import { untrack } from 'svelte';

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
	groups = $state<Group[]>([]);

	/** Every row on screen, in the order their owners mounted; a key listed twice keeps its first meaning. */
	get rows(): KeyHint[] {
		const seen = new Set<string>();
		const out: KeyHint[] = [];
		for (const group of this.groups) {
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
		return this.groups.some((g) => g.rows.some((r) => r.keys.length === 1 && r.keys[0] === key));
	}

	// Called while components mount and unmount: reading the list there must
	// not make whatever is rendering depend on it.
	open(): Group {
		const group = new Group();
		untrack(() => (this.groups = [...this.groups, group]));
		return group;
	}

	close(group: Group): void {
		untrack(() => (this.groups = this.groups.filter((g) => g !== group)));
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
