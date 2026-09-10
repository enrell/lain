/*
 * One place touches localStorage. Components and stores ask this
 * module; nothing else reads browser storage directly.
 */

const TOKEN_KEY = 'lain.token';

function store(): Storage | null {
	try {
		return globalThis.localStorage ?? null;
	} catch {
		return null;
	}
}

export const tokenStorage = {
	load(): string | null {
		return store()?.getItem(TOKEN_KEY) ?? null;
	},
	save(token: string): void {
		store()?.setItem(TOKEN_KEY, token);
	},
	clear(): void {
		store()?.removeItem(TOKEN_KEY);
	}
};

/** Small namespaced preference helper (player volume etc.). */
export const prefs = {
	get(key: string, fallback: string | null = null): string | null {
		return store()?.getItem(`lain.${key}`) ?? fallback;
	},
	set(key: string, value: string): void {
		store()?.setItem(`lain.${key}`, value);
	},
	getNumber(key: string, fallback: number): number {
		const raw = this.get(key);
		if (raw === null) return fallback;
		const n = Number(raw);
		return Number.isFinite(n) ? n : fallback;
	},
	getBool(key: string, fallback: boolean): boolean {
		const raw = this.get(key);
		if (raw === null) return fallback;
		return raw === 'true';
	}
};
