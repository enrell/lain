/*
 * One place touches localStorage. Components and stores ask this
 * module; nothing else reads browser storage directly.
 */
import { mascotById } from '$lib/profile/mascots';

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

/**
 * Accounts that have signed in on this browser (D-297). A tile needs
 * the id, the username, the display name and an avatar reference
 * (a mascot id, or `upload` which the login page draws as initials
 * because the picture needs a session). Never a token, a password
 * or a PIN: those fields are dropped on the way in and on the way out.
 */
export interface RememberedAvatar {
	kind: '' | 'mascot' | 'upload';
	mascot?: string;
}

export interface RememberedAccount {
	id: string;
	username: string;
	displayName: string;
	avatar: RememberedAvatar;
}

const ACCOUNTS_KEY = 'lain.accounts';

function readRaw(key: string): string | null {
	try {
		return store()?.getItem(key) ?? null;
	} catch {
		return null;
	}
}

function writeRaw(key: string, value: string): void {
	try {
		store()?.setItem(key, value);
	} catch {
		// Full or blocked storage: sign-in still works, the tile is missing.
	}
}

function removeRaw(key: string): void {
	try {
		store()?.removeItem(key);
	} catch {
		// Same as a write that did not land.
	}
}

function cleanAvatar(raw: unknown): RememberedAvatar {
	if (!raw || typeof raw !== 'object') return { kind: '' };
	const kind = (raw as { kind?: unknown }).kind;
	const mascot = (raw as { mascot?: unknown }).mascot;
	if (kind === 'mascot' && typeof mascot === 'string' && mascotById(mascot)) {
		return { kind: 'mascot', mascot };
	}
	if (kind === 'upload') return { kind: 'upload' };
	return { kind: '' };
}

function usable(value: string, max: number): string | null {
	const trimmed = value.trim();
	if (!trimmed || trimmed.length > max || /[\u0000-\u001f]/.test(trimmed)) return null;
	return trimmed;
}

function cleanAccount(raw: unknown): RememberedAccount | null {
	if (!raw || typeof raw !== 'object') return null;
	const row = raw as { id?: unknown; username?: unknown; displayName?: unknown; avatar?: unknown };
	if (typeof row.id !== 'string' || typeof row.username !== 'string') return null;
	const id = usable(row.id, 128);
	const username = usable(row.username, 80);
	if (!id || !username) return null;
	let displayName = typeof row.displayName === 'string' ? usable(row.displayName, 80) : null;
	if (!displayName) displayName = username;
	return { id, username, displayName, avatar: cleanAvatar(row.avatar) };
}

/** Tiles for this browser, newest sign-in first. Corrupt storage is an empty list. */
export function loadAccounts(): RememberedAccount[] {
	const raw = readRaw(ACCOUNTS_KEY);
	if (raw === null || raw === '') return [];
	let parsed: unknown;
	try {
		parsed = JSON.parse(raw);
	} catch {
		removeRaw(ACCOUNTS_KEY);
		return [];
	}
	if (!Array.isArray(parsed)) {
		removeRaw(ACCOUNTS_KEY);
		return [];
	}
	const accounts: RememberedAccount[] = [];
	const seen = new Set<string>();
	for (const item of parsed) {
		const account = cleanAccount(item);
		if (!account || seen.has(account.id)) continue;
		seen.add(account.id);
		accounts.push(account);
	}
	const cleaned = JSON.stringify(accounts);
	if (cleaned !== raw) writeRaw(ACCOUNTS_KEY, cleaned);
	return accounts;
}

/**
 * Remembers a successful sign-in. The same id is updated and moved
 * first. Anything other than the tile fields is ignored, so a caller
 * that hands over a whole user record still cannot persist a secret.
 */
export function rememberAccount(input: {
	id: string;
	username: string;
	displayName?: string | null;
	avatar?: { kind?: string | null; mascot?: string | null } | null;
}): void {
	const account = cleanAccount({
		id: input.id,
		username: input.username,
		displayName: input.displayName ?? '',
		avatar: input.avatar ?? undefined
	});
	if (!account) return;
	const next = [account, ...loadAccounts().filter((a) => a.id !== account.id)];
	writeRaw(ACCOUNTS_KEY, JSON.stringify(next));
}

/** Drops one tile. The account on the server is untouched. */
export function forgetAccount(id: string): void {
	writeRaw(ACCOUNTS_KEY, JSON.stringify(loadAccounts().filter((a) => a.id !== id)));
}

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
