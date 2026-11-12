/*
 * Whether the work on screen is one of the account's favourites: one
 * answer, shared by the heart beside a title's main action and the
 * social block's F key, so the two can never disagree. The social
 * block feeds it when it loads the work.
 */
import { api } from '$lib/api';
import { targetOf, type WorkRef } from '$lib/api/social';
import { workKey } from './format';

class FavoriteState {
	on = $state(false);
	busy = $state(false);
	/** False until the work is known (and on a server without the social module). */
	ready = $state(false);
	#work: WorkRef | null = null;

	set(work: WorkRef, favorites: WorkRef[]): void {
		this.#work = work;
		this.on = favorites.some((f) => workKey(f) === workKey(work));
		this.ready = true;
	}

	clear(): void {
		this.#work = null;
		this.ready = false;
		this.on = false;
	}

	/** Flips the favourite and resolves to the new state; rejects when the server refuses. */
	async toggle(): Promise<boolean> {
		const work = this.#work;
		if (!work || this.busy) return this.on;
		this.busy = true;
		try {
			const key = workKey(work);
			const st = await api.social.settings();
			const rest = st.favorites.filter((f) => workKey(f) !== key).map(targetOf);
			const saved = await api.social.updateSettings({ favorites: this.on ? rest : [...rest, targetOf(work)] });
			this.on = saved.favorites.some((f) => workKey(f) === key);
			return this.on;
		} finally {
			this.busy = false;
		}
	}
}

export const favoriteState = new FavoriteState();
