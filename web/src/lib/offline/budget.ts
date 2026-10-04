/*
 * How much this browser may keep offline. Two limits apply: the user's
 * own cap (a setting, default 1 GiB, 0 = only the browser's quota) and
 * the browser's quota as navigator.storage.estimate() reports it, of
 * which we leave a fifth free for everything else the origin stores.
 */

export const DEFAULT_CAP = 1024 ** 3;
const CAP_KEY = 'lain.offline.capBytes';
const QUOTA_SHARE = 0.8;

export interface Estimate {
	usage?: number;
	quota?: number;
}

/**
 * The byte budget for saved items. `ours` is what saved items already
 * use; other origin data (app shell, caches) counts against the quota.
 */
export function effectiveBudget(cap: number, est: Estimate, ours: number): number {
	let budget = cap > 0 ? cap : Number.POSITIVE_INFINITY;
	if (est.quota && est.quota > 0) {
		const others = Math.max(0, (est.usage ?? 0) - ours);
		budget = Math.min(budget, Math.max(0, est.quota * QUOTA_SHARE - others));
	}
	return budget;
}

/** Whether `need` more bytes fit next to `ours` within `budget`. */
export function fits(need: number, ours: number, budget: number): boolean {
	return ours + need <= budget;
}

type KV = Pick<Storage, 'getItem' | 'setItem'>;

function store(): KV | null {
	try {
		return globalThis.localStorage ?? null;
	} catch {
		return null;
	}
}

export function loadCap(kv: KV | null = store()): number {
	try {
		const raw = kv?.getItem(CAP_KEY) ?? null;
		if (raw === null) return DEFAULT_CAP;
		const n = Number(raw);
		return Number.isFinite(n) && n >= 0 ? n : DEFAULT_CAP;
	} catch {
		return DEFAULT_CAP;
	}
}

export function saveCap(bytes: number, kv: KV | null = store()): void {
	try {
		kv?.setItem(CAP_KEY, String(Math.max(0, Math.round(bytes))));
	} catch {
		// Private mode or blocked storage: the default applies.
	}
}
