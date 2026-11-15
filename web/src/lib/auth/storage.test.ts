import { afterEach, describe, expect, it, vi } from 'vitest';
import { forgetAccount, loadAccounts, rememberAccount } from './storage';

function memoryStorage(): Map<string, string> {
	const values = new Map<string, string>();
	vi.stubGlobal('localStorage', {
		getItem: (key: string) => values.get(key) ?? null,
		setItem: (key: string, value: string) => {
			values.set(key, value);
		},
		removeItem: (key: string) => {
			values.delete(key);
		}
	});
	return values;
}

afterEach(() => vi.unstubAllGlobals());

describe('remembered accounts', () => {
	it('adds a tile and dedupes by id, newest first', () => {
		memoryStorage();
		rememberAccount({ id: 'user-ada', username: 'ada', displayName: 'Ada' });
		rememberAccount({
			id: 'user-bea',
			username: 'bea',
			displayName: 'Bea',
			avatar: { kind: 'mascot', mascot: 'wired' }
		});
		rememberAccount({
			id: 'user-ada',
			username: 'ada',
			displayName: 'Ada Lovelace',
			avatar: { kind: 'upload', mascot: 'wired' }
		});
		expect(loadAccounts()).toEqual([
			{ id: 'user-ada', username: 'ada', displayName: 'Ada Lovelace', avatar: { kind: 'upload' } },
			{ id: 'user-bea', username: 'bea', displayName: 'Bea', avatar: { kind: 'mascot', mascot: 'wired' } }
		]);
	});

	it('forgets one tile and leaves the others', () => {
		memoryStorage();
		rememberAccount({ id: 'user-ada', username: 'ada', displayName: 'Ada' });
		rememberAccount({ id: 'user-bea', username: 'bea', displayName: 'Bea' });
		forgetAccount('user-ada');
		expect(loadAccounts().map((a) => a.id)).toEqual(['user-bea']);
		forgetAccount('missing');
		expect(loadAccounts().map((a) => a.id)).toEqual(['user-bea']);
	});

	it('drops a corrupt record and keeps a usable one', () => {
		const values = memoryStorage();
		values.set(
			'lain.accounts',
			JSON.stringify([
				{ id: 'user-ada', username: 'ada', displayName: 'Ada', avatar: { kind: 'mascot', mascot: 'nope', version: 3 } },
				{ id: '', username: 'ghost' },
				'nope',
				{ id: 'user-ada', username: 'ada-again', displayName: 'Duplicate' },
				{ username: 'missing-id' }
			])
		);
		expect(loadAccounts()).toEqual([
			{ id: 'user-ada', username: 'ada', displayName: 'Ada', avatar: { kind: '' } }
		]);
	});

	it('treats garbage, a non-array and a thrown store as no tiles', () => {
		const values = memoryStorage();
		values.set('lain.accounts', '{not json');
		expect(loadAccounts()).toEqual([]);
		expect(values.has('lain.accounts')).toBe(false);
		values.set('lain.accounts', JSON.stringify({ token: 'sekret' }));
		expect(loadAccounts()).toEqual([]);
		expect(values.has('lain.accounts')).toBe(false);
		vi.stubGlobal('localStorage', {
			getItem: () => {
				throw new Error('denied');
			},
			setItem: () => {
				throw new Error('denied');
			},
			removeItem: () => {
				throw new Error('denied');
			}
		});
		expect(loadAccounts()).toEqual([]);
		expect(() => rememberAccount({ id: 'user-ada', username: 'ada' })).not.toThrow();
		expect(() => forgetAccount('user-ada')).not.toThrow();
	});

	it('never writes a token, a password, a PIN or an avatar version', () => {
		const values = memoryStorage();
		const dirty = {
			id: 'user-ada',
			username: 'ada',
			displayName: 'Ada',
			token: 'sekret-token',
			password: 'hunter2',
			pin: '9876',
			avatar: { kind: 'mascot', mascot: 'moth', version: 9 }
		};
		rememberAccount(dirty);
		const raw = values.get('lain.accounts') ?? '';
		expect(raw).not.toContain('sekret-token');
		expect(raw).not.toContain('hunter2');
		expect(raw).not.toContain('9876');
		expect(raw).not.toMatch(/token|password|pin|pwd|version/i);
		expect(JSON.parse(raw)).toEqual([
			{ id: 'user-ada', username: 'ada', displayName: 'Ada', avatar: { kind: 'mascot', mascot: 'moth' } }
		]);
		expect(values.has('lain.token')).toBe(false);
	});

	it('refuses an empty id and falls back to the username for a blank name', () => {
		const values = memoryStorage();
		rememberAccount({ id: ' ', username: 'ada', displayName: 'Ada', password: 'nope' } as never);
		expect(values.has('lain.accounts')).toBe(false);
		rememberAccount({ id: 'user-ada', username: 'ada', displayName: '   ' });
		expect(loadAccounts()[0]?.displayName).toBe('ada');
	});
});
