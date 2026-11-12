<script lang="ts">
	import { t } from '$lib/i18n';
	/*
	 * The one row pattern of Settings: label and a one-line hint on the
	 * left, the control on the right, inside a group's panel (which gives
	 * the inner padding and the hairline between rows). The control column
	 * has one width on every row, so selects and fields share both edges
	 * down the page and every hint wraps at the same place; switches,
	 * numbers and buttons sit at its right edge. `id` is the anchor Ctrl+K
	 * jumps to.
	 */
	import type { Snippet } from 'svelte';
	import Check from '@lucide/svelte/icons/check';

	let {
		id,
		label,
		hint,
		saved = false,
		stack = false,
		children,
		below
	}: {
		id?: string;
		label: string;
		hint?: string;
		/** Flashes "saved" after an instant-apply change. */
		saved?: boolean;
		/** Put the control under the label (wide controls: tables, grids). */
		stack?: boolean;
		children?: Snippet;
		below?: Snippet;
	} = $props();
</script>

<div
	{id}
	data-setting-row
	class="setting-row scroll-mt-28 py-4 {stack
		? 'space-y-3'
		: 'grid gap-x-8 gap-y-3 sm:min-h-16 sm:grid-cols-[minmax(0,1fr)_minmax(0,16rem)] sm:items-center lg:grid-cols-[minmax(0,1fr)_minmax(0,20rem)]'}"
>
	<div class="min-w-0">
		<p class="flex items-center gap-2 text-sm font-medium text-foreground">
			{label}
			{#if saved}
				<span class="saved-flash inline-flex items-center gap-1 font-mono text-[10px] uppercase tracking-[0.14em] text-accent" role="status">
					<Check class="size-3" /> {t('settings.saved')}
				</span>
			{/if}
		</p>
		{#if hint}<p class="mt-0.5 text-[13px] leading-5 text-muted">{hint}</p>{/if}
	</div>
	{#if children}
		<div class="flex min-w-0 flex-wrap items-center gap-2 {stack ? '' : 'sm:justify-end'}">
			{@render children()}
		</div>
	{/if}
	{#if below}
		<!-- Hidden while the snippet renders nothing (an error that is not
		     there): an empty grid row still took the row gap. -->
		<div class="min-w-0 empty:hidden sm:col-span-2">{@render below()}</div>
	{/if}
</div>
