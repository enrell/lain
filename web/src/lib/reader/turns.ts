import { progressFor } from './layout';

export type PageSnapshot = ReturnType<typeof progressFor>;

export interface PendingTurn {
	id: string;
	snap: PageSnapshot;
}

/**
 * Which page turns the reader owes the server, kept apart from the
 * component so the rules are testable without a browser.
 *
 * - Opening a file reports nothing: a finished volume opens on page 1 and
 *   must not be reset by being looked at. Once the reader has turned a
 *   page, every page counts, the opened one included (turning on and then
 *   back again is a real place).
 * - The newest turn stays owed until the write carrying exactly it is
 *   known to have landed. Closing the tab never runs the component's
 *   teardown, so `take()` hands that turn to a keepalive write as the page
 *   hides; a turn already saved is not sent twice.
 */
export class PageTurns {
	#opened = -1;
	#moved = false;
	#pending: PendingTurn | null = null;

	/**
	 * A file opened on this page. A turn still owed for the file before it
	 * (the next volume opened inside the reader) stays owed.
	 */
	open(page: number): void {
		this.#opened = page;
		this.#moved = false;
	}

	/** A page came into view; returns the snapshot to report, or null when there is none. */
	turn(id: string, page: number, total: number): PageSnapshot | null {
		if (!this.#moved && page === this.#opened) return null;
		this.#moved = true;
		const snap = progressFor(page, total);
		this.#pending = { id, snap };
		return snap;
	}

	/** A write landed: the turn it carried is no longer owed (a newer one still is). */
	saved(id: string, snap: PageSnapshot): void {
		const pending = this.#pending;
		if (
			pending &&
			pending.id === id &&
			pending.snap.position_sec === snap.position_sec &&
			pending.snap.duration_sec === snap.duration_sec &&
			pending.snap.completed === snap.completed
		) {
			this.#pending = null;
		}
	}

	/** The turn still owed, handed over once (the page is hiding). */
	take(): PendingTurn | null {
		const pending = this.#pending;
		this.#pending = null;
		return pending;
	}
}
