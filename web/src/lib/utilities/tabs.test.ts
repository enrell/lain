import { describe, expect, it } from 'vitest';
import { tabStep } from './tabs';

describe('tabStep', () => {
	const tabs = ['feed', 'friends', 'notifications'] as const;

	it('steps right and left, wrapping at the ends', () => {
		expect(tabStep('ArrowRight', tabs, 'feed')).toBe('friends');
		expect(tabStep('ArrowRight', tabs, 'notifications')).toBe('feed');
		expect(tabStep('ArrowLeft', tabs, 'feed')).toBe('notifications');
	});

	it('jumps to the ends', () => {
		expect(tabStep('Home', tabs, 'notifications')).toBe('feed');
		expect(tabStep('End', tabs, 'feed')).toBe('notifications');
	});

	it('ignores other keys and unknown tabs', () => {
		expect(tabStep('ArrowDown', tabs, 'feed')).toBeNull();
		expect(tabStep('ArrowRight', tabs, 'other' as never)).toBeNull();
	});
});
