import { ApiError, type UserSummary, type WorkRef } from '$lib/api';
import type { User } from '$lib/api/types';
import { t } from '$lib/i18n';
import { isTypingTarget } from '$lib/utilities/guards';
import { errorMessage } from '$lib/utilities/errors';

/** The account's display name, its username, or "removed account". */
export function userName(u: UserSummary | undefined | null): string {
	return u?.display_name?.trim() || u?.username || t('social.removedUser');
}

/** Avatar takes the account shape; a summary carries the same fields flat. */
export function asAvatarUser(u: UserSummary): Pick<User, 'id' | 'username' | 'profile'> {
	return { id: u.id, username: u.username, profile: { display_name: u.display_name, avatar: u.avatar ?? {} } };
}

/** Where a work opens: its catalog page when one exists, else a search. */
export function workHref(w: WorkRef): string {
	return w.item_id ? `/item/${encodeURIComponent(w.item_id)}` : `/search?q=${encodeURIComponent(w.title)}`;
}

/** The same identity the server uses: kind + collapsed lowercase title. */
export function workKey(w: Pick<WorkRef, 'kind' | 'title'>): string {
	return `${w.kind.toLowerCase()}\u0000${w.title.toLowerCase().split(/\s+/).filter(Boolean).join(' ')}`;
}

const KINDS: Record<string, () => string> = {
	anime: () => t('social.kind.anime'),
	series: () => t('social.kind.series'),
	movie: () => t('social.kind.movie'),
	manga: () => t('social.kind.manga'),
	comic: () => t('social.kind.comic')
};

/** A known media kind in the interface language; unknown kinds stay as their token. */
export function kindLabel(kind: string): string {
	return KINDS[kind]?.() ?? kind;
}

/**
 * A single-key page shortcut, or '' when the key belongs to something
 * else: a text field, an open dialog, a modifier chord.
 */
export function pageKey(e: KeyboardEvent): string {
	if (e.ctrlKey || e.metaKey || e.altKey || e.defaultPrevented) return '';
	if (isTypingTarget(e.target) || document.querySelector('[role="dialog"]')) return '';
	return e.key;
}

/** Social errors keep the server's reason even for 403 ("friends only"). */
export function socialError(err: unknown, fallback: string): string {
	if (err instanceof ApiError && err.kind === 'forbidden' && err.message) return err.message;
	return errorMessage(err, fallback);
}
