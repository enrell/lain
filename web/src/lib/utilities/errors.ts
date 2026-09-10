import { ApiError } from '$lib/api';

/**
 * Product-level message for any thrown value. Server validation and
 * provider messages are kept (they are written for humans); transport
 * and auth failures get context the server cannot give.
 */
export function errorMessage(err: unknown, fallback = 'Something went wrong.'): string {
	if (err instanceof ApiError) {
		switch (err.kind) {
			case 'network':
				return 'Cannot reach the Lain server. Is it running?';
			case 'unauthorized':
				return err.message && err.message !== 'unauthorized'
					? err.message
					: 'Your session expired. Sign in again.';
			case 'forbidden':
				return 'You do not have permission for that.';
			case 'conflict':
				return err.message || 'Someone else changed this first. Reload and try again.';
			case 'unavailable':
				return err.message || 'That service is temporarily unavailable.';
			default:
				return err.message || fallback;
		}
	}
	if (err instanceof DOMException && err.name === 'AbortError') return 'Request cancelled.';
	if (err instanceof Error && err.message) return err.message;
	return fallback;
}
