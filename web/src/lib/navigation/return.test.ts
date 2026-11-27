import { beforeEach, describe, expect, it, vi } from 'vitest';

const goto = vi.fn(async () => {});
vi.mock('$app/navigation', () => ({ goto: (...a: unknown[]) => goto(...(a as [])) }));

const { isPlayback, leavePlayback, listFallback, noteNavigation, resetNavigation } = await import('./return');

const u = (p: string) => new URL(`https://lain.test${p}`);

describe('return', () => {
	beforeEach(() => {
		resetNavigation();
		goto.mockClear();
	});

	it('knows playback pages', () => {
		expect(isPlayback('/player/a')).toBe(true);
		expect(isPlayback('/read/a')).toBe(true);
		expect(isPlayback('/item/a')).toBe(false);
	});

	it('goes back in history to the title page the player was opened from', () => {
		const back = vi.fn();
		vi.stubGlobal('history', { back, length: 3 });
		noteNavigation(u('/library'), u('/item/a'));
		noteNavigation(u('/item/a'), u('/player/a'));
		noteNavigation(u('/player/a'), u('/player/b')); // the next episode
		leavePlayback('/item/b');
		expect(back).toHaveBeenCalledOnce();
		expect(goto).not.toHaveBeenCalled();
		vi.unstubAllGlobals();
	});

	it('replaces the player with the title page when opened from elsewhere', () => {
		noteNavigation(u('/'), u('/player/a'));
		leavePlayback('/item/a');
		expect(goto).toHaveBeenCalledWith('/item/a', { replaceState: true });
	});

	it('remembers the last list page, not titles or players', () => {
		expect(listFallback()).toBe('/library');
		noteNavigation(null, u('/library?lib=x'));
		noteNavigation(u('/library?lib=x'), u('/item/a'));
		noteNavigation(u('/item/a'), u('/player/a'));
		noteNavigation(u('/player/a'), u('/item/a'));
		expect(listFallback()).toBe('/library?lib=x');
	});
});
