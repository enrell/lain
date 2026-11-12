<script lang="ts">
	/*
	 * Left and Right along a rail of cards (Home's rows, the library's
	 * shelves, "More in"), Home and End to its ends: Tab alone walked every
	 * card of every rail. A rail is any sideways-scrolling row; the card is
	 * its child holding the focus, and the next one's link takes the focus,
	 * scrolled into view. Grids with keys of their own (the library, a
	 * title's episodes) handle the arrows before this and are left alone.
	 */
	import { isTypingTarget } from '$lib/utilities/guards';

	const KEYS = new Set(['ArrowLeft', 'ArrowRight', 'Home', 'End']);

	function railOf(el: HTMLElement): { rail: HTMLElement; card: HTMLElement } | null {
		let card = el;
		for (let node = el.parentElement; node && node !== document.body; node = node.parentElement) {
			const style = getComputedStyle(node);
			if ((style.overflowX === 'auto' || style.overflowX === 'scroll') && style.display.includes('flex')) return { rail: node, card };
			card = node;
		}
		return null;
	}

	function onKey(e: KeyboardEvent): void {
		if (!KEYS.has(e.key) || e.defaultPrevented || e.ctrlKey || e.metaKey || e.altKey || e.shiftKey) return;
		const el = document.activeElement;
		if (!(el instanceof HTMLElement) || isTypingTarget(el) || document.querySelector('[role="dialog"]')) return;
		if (el.closest('[role="tablist"], [role="menu"], [role="listbox"], [role="slider"], nav')) return;
		const found = railOf(el);
		if (!found) return;
		const cards = [...found.rail.children] as HTMLElement[];
		const at = cards.indexOf(found.card);
		if (at < 0) return;
		const to = e.key === 'Home' ? 0 : e.key === 'End' ? cards.length - 1 : at + (e.key === 'ArrowRight' ? 1 : -1);
		const link = cards[to]?.querySelector<HTMLElement>('a[href], button');
		if (!link || to === at) return;
		e.preventDefault();
		link.focus({ preventScroll: true });
		link.scrollIntoView({ block: 'nearest', inline: 'nearest' });
	}
</script>

<svelte:window onkeydown={onKey} />
