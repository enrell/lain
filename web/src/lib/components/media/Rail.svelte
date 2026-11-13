<script lang="ts">
	/*
	 * A row of cards that scrolls sideways (Home's rows, the library's
	 * shelves, "More in"): it runs through the page's side padding to the
	 * window's edge, so the card that continues is cut by the window, and
	 * its first card and scroll stops stay on the gutter.
	 *
	 * With a mouse, a button at each end pages it while that side hides
	 * more, shown on hover: a wheel without a sideways axis could not move
	 * it. Keys walk it card by card (RailKeys), so the buttons stay out of
	 * the Tab order, and a touch screen swipes.
	 */
	import type { Snippet } from 'svelte';
	import ChevronLeft from '@lucide/svelte/icons/chevron-left';
	import ChevronRight from '@lucide/svelte/icons/chevron-right';
	import { t } from '$lib/i18n';
	import { scrollEdges, type Edges } from '$lib/utilities/scroll-edges';

	let { class: className = '', children }: { /** Added to the row: card sizes (RAIL), gaps. */ class?: string; children: Snippet } = $props();

	let row = $state<HTMLElement>();
	let edges = $state<Edges>({ start: false, end: false });
	// Level with the middle of the cards' pictures, not of picture and caption.
	let middle = $state<number | null>(null);

	function measure(): void {
		const art = row?.querySelector<HTMLElement>('[class*="aspect-"]');
		if (!row || !art) return;
		const box = art.getBoundingClientRect();
		middle = box.top - row.getBoundingClientRect().top + box.height / 2;
	}
	$effect(() => {
		if (!row) return;
		const observer = new ResizeObserver(measure);
		observer.observe(row);
		return () => observer.disconnect();
	});

	function page(direction: 1 | -1): void {
		if (!row) return;
		const rtl = getComputedStyle(row).direction === 'rtl';
		const calm = window.matchMedia('(prefers-reduced-motion: reduce)').matches;
		// Most of a screen, so the card cut at the edge comes in whole; the snap points square it.
		row.scrollBy({ left: direction * (rtl ? -1 : 1) * row.clientWidth * 0.85, behavior: calm ? 'auto' : 'smooth' });
	}
</script>

{#snippet button(direction: 1 | -1)}
	<button
		type="button"
		tabindex="-1"
		aria-label={direction < 0 ? t('rail.previous') : t('rail.next')}
		class={[
			'absolute z-10 hidden size-10 -translate-y-1/2 items-center justify-center rounded-full bg-background/80 text-foreground opacity-0 shadow-[0_8px_24px_rgba(0,0,0,0.45)] ring-1 ring-white/15 backdrop-blur transition-opacity duration-150 hover:bg-surface-hover group-hover/rail:opacity-100 pointer-fine:flex',
			direction < 0 ? 'start-2 lg:start-3' : 'end-2 lg:end-3'
		].join(' ')}
		style:top={middle === null ? '40%' : `${middle}px`}
		onclick={() => page(direction)}
	>
		{#if direction < 0}<ChevronLeft class="size-5 rtl:rotate-180" aria-hidden="true" />{:else}<ChevronRight class="size-5 rtl:rotate-180" aria-hidden="true" />{/if}
	</button>
{/snippet}

<div class="group/rail relative -mx-5 sm:-mx-8 lg:-mx-10">
	<div
		bind:this={row}
		class={['flex snap-x scroll-px-5 gap-3.5 overflow-x-auto px-5 pb-4 no-scrollbar sm:scroll-px-8 sm:px-8 md:gap-4 lg:scroll-px-10 lg:px-10', className].join(' ')}
		{@attach scrollEdges((next) => {
			edges = next;
			measure();
		})}
	>
		{@render children()}
	</div>
	{#if edges.start}{@render button(-1)}{/if}
	{#if edges.end}{@render button(1)}{/if}
</div>
