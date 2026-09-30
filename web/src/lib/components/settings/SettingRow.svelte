<script lang="ts">
	/*
	 * The one row pattern of Settings: label and a one-line hint on the
	 * left, the control on the right, a hairline below. One vertical scan
	 * line for the eye; `id` is the anchor Ctrl+K jumps to.
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
	class="setting-row scroll-mt-28 border-b border-hairline py-4 {stack
		? 'space-y-3'
		: 'grid gap-x-8 gap-y-2 sm:grid-cols-[minmax(0,17rem)_minmax(0,1fr)] sm:items-center'}"
>
	<div class="min-w-0">
		<p class="flex items-center gap-2 text-sm font-medium text-foreground">
			{label}
			{#if saved}
				<span class="saved-flash inline-flex items-center gap-1 font-mono text-[10px] uppercase tracking-[0.14em] text-accent" role="status">
					<Check class="size-3" /> saved
				</span>
			{/if}
		</p>
		{#if hint}<p class="mt-0.5 text-xs leading-5 text-muted">{hint}</p>{/if}
	</div>
	{#if children}
		<div class="flex min-w-0 flex-wrap items-center gap-2 {stack ? '' : 'sm:justify-end'}">
			{@render children()}
		</div>
	{/if}
	{#if below}
		<div class="min-w-0 sm:col-span-2">{@render below()}</div>
	{/if}
</div>
