import { describe, expect, it } from 'vitest';
import { ApiError } from '$lib/api/client';
import { signInRefusal } from './sign-in';

describe('signInRefusal', () => {
	it('reads a 401 as a wrong pair, whatever its wording', () => {
		expect(signInRefusal(new ApiError('unauthorized', 'invalid credentials', 401))).toBe('mismatch');
		expect(signInRefusal(new ApiError('unauthorized', 'wrong username or password', 401))).toBe('mismatch');
	});
	it('tells a disabled account apart: retyping the password cannot help', () => {
		expect(signInRefusal(new ApiError('unauthorized', 'account disabled', 401))).toBe('disabled');
	});
	it('leaves every other failure to the generic message', () => {
		expect(signInRefusal(new ApiError('server', 'internal error', 500))).toBeNull();
		expect(signInRefusal(new ApiError('network', 'offline', 0))).toBeNull();
		expect(signInRefusal(new Error('invalid credentials'))).toBeNull();
	});
});
