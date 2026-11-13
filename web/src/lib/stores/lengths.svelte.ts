import { SvelteMap } from 'svelte/reactivity';
import { api } from '$lib/api';

/*
 * How long single videos are (D-276), for the cards that have nothing
 * else to say under their title: a film or a clip without a year was
 * captioned with its raw kind ("video"). Asked for only for those cards,
 * in one request for the cards drawn together, once per item a visit; the
 * server probes a file once and keeps the answer.
 */
const lengths = new SvelteMap<string, number>();
const asked = new Set<string>();
let queue: string[] = [];
let timer: ReturnType<typeof setTimeout> | undefined;

/** Asks for an item's length; it arrives in `lengthOf`, or never when the server cannot tell. */
export function wantLength(id: string): void {
	if (asked.has(id)) return;
	asked.add(id);
	queue.push(id);
	timer ??= setTimeout(flush, 30);
}

function flush(): void {
	const ids = queue;
	queue = [];
	timer = undefined;
	void api.playback.durations(ids).then((found) => {
		for (const [id, sec] of found) lengths.set(id, sec);
	});
}

/** The item's length in seconds, once known. */
export function lengthOf(id: string): number | undefined {
	return lengths.get(id);
}
