import { api } from '$lib/api';

/*
 * Unread notification count for the navigation badge. The server has no
 * push channel (docs/slices/social.md), so the shell polls while a
 * session is open; pages that change notifications call refresh().
 */
class SocialBadge {
	unread = $state(0);
	#timer: ReturnType<typeof setInterval> | undefined;

	async refresh(): Promise<void> {
		try {
			const res = await api.social.notifications({ suppressAuthRedirect: true });
			this.unread = res.unread;
		} catch {
			// A missed poll keeps the last count; the next one retries.
		}
	}

	start(intervalMs = 60_000): void {
		this.stop();
		void this.refresh();
		this.#timer = setInterval(() => void this.refresh(), intervalMs);
	}

	stop(): void {
		clearInterval(this.#timer);
		this.#timer = undefined;
		this.unread = 0;
	}
}

export const socialBadge = new SocialBadge();
