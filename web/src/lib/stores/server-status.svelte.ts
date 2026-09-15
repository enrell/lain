import { request } from '$lib/api/client';

export type ServerState = 'checking' | 'online' | 'offline';

/**
 * Server reachability for the sidebar status pill. The health endpoint
 * needs no auth, so this works before login too. Anything else keeps
 * using its own error states; this store only answers "is it up?".
 */
class ServerStatusStore {
	state = $state<ServerState>('checking');
	private timer: ReturnType<typeof setInterval> | null = null;
	private onOnline = () => void this.refresh();
	private onOffline = () => {
		this.state = 'offline';
	};

	async refresh(): Promise<void> {
		try {
			await request<{ status: string }>('/api/health');
			this.state = 'online';
		} catch {
			this.state = 'offline';
		}
	}

	start(): void {
		if (this.timer || typeof window === 'undefined') return;
		void this.refresh();
		this.timer = setInterval(() => void this.refresh(), 30000);
		window.addEventListener('online', this.onOnline);
		window.addEventListener('offline', this.onOffline);
	}

	stop(): void {
		if (this.timer) clearInterval(this.timer);
		this.timer = null;
		if (typeof window !== 'undefined') {
			window.removeEventListener('online', this.onOnline);
			window.removeEventListener('offline', this.onOffline);
		}
	}
}

export const serverStatus = new ServerStatusStore();
