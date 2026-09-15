import { theme as themeApi } from '$lib/api/theme';
import type { ThemePalette } from '$lib/api/types';

const refreshMs = 3000;

const tokenKeys = {
	background: '--color-background',
	surface: '--color-surface',
	surface_hover: '--color-surface-hover',
	surface_active: '--color-surface-active',
	foreground: '--color-foreground',
	muted: '--color-muted',
	line: '--color-line',
	accent: '--color-accent',
	accent_hover: '--color-accent-hover',
	accent_foreground: '--color-accent-fg',
	danger: '--color-danger',
	danger_foreground: '--color-danger-fg',
	success: '--color-success',
	warning: '--color-warning'
} as const;

export function themeVariables(palette: ThemePalette): Record<string, string> {
	return Object.fromEntries(
		Object.entries(tokenKeys).map(([field, token]) => [token, palette[field as keyof typeof tokenKeys]])
	);
}

function applyTheme(palette: ThemePalette): void {
	const root = document.documentElement;
	for (const [token, value] of Object.entries(themeVariables(palette))) {
		root.style.setProperty(token, value);
	}
	root.style.colorScheme = palette.mode;
}

class AppearanceStore {
	private timer: ReturnType<typeof setInterval> | null = null;
	private controller: AbortController | null = null;
	private last = '';
	private onVisible = () => {
		if (document.visibilityState === 'visible') void this.refresh();
	};

	async refresh(): Promise<void> {
		if (this.controller) return;
		this.controller = new AbortController();
		try {
			const palette = await themeApi.get(this.controller.signal);
			const revision = JSON.stringify(palette);
			if (revision !== this.last) {
				applyTheme(palette);
				this.last = revision;
			}
		} catch (error) {
			if (!(error instanceof DOMException && error.name === 'AbortError') && import.meta.env.DEV) {
				console.debug('[theme] keeping built-in palette', error);
			}
		} finally {
			this.controller = null;
		}
	}

	start(): void {
		if (this.timer || typeof document === 'undefined') return;
		void this.refresh();
		this.timer = setInterval(() => void this.refresh(), refreshMs);
		document.addEventListener('visibilitychange', this.onVisible);
	}

	stop(): void {
		if (this.timer) clearInterval(this.timer);
		this.timer = null;
		this.controller?.abort();
		this.controller = null;
		if (typeof document !== 'undefined') {
			document.removeEventListener('visibilitychange', this.onVisible);
		}
	}
}

export const appearance = new AppearanceStore();
