/*
 * Why the server refused a sign-in, as the form tells it. POST
 * /api/auth/login answers 401 for two reasons, both pinned by
 * conformance/cases/fresh/40-users.json: a wrong pair ('invalid
 * credentials') and a disabled account ('account disabled'). A wrong pair
 * is told by the status, so a reworded message still reads as one; a
 * disabled account shares the status and is told by its pinned message.
 * Read as a mistyped password, it kept the person retyping one that was
 * right. A username the server no longer has is 404 'unknown user'
 * (the account was removed); that is not a wrong password.
 */
import { ApiError } from '$lib/api/client';

export type SignInRefusal = 'mismatch' | 'disabled' | 'unknown';

/** The refusal behind a failed sign-in, or null for any other failure (network, server). */
export function signInRefusal(err: unknown): SignInRefusal | null {
	if (!(err instanceof ApiError)) return null;
	if (err.status === 404 && err.message === 'unknown user') return 'unknown';
	if (err.status !== 401) return null;
	return err.message === 'account disabled' ? 'disabled' : 'mismatch';
}
