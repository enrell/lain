<script lang="ts">
	import type { Snippet } from 'svelte';
	import Check from '@lucide/svelte/icons/check';

	let {
		label,
		active = false,
		disabled = false,
		check = false,
		onclick,
		children
	}: {
		label: string;
		active?: boolean;
		disabled?: boolean;
		/**
		 * An option of the toggle before it (the spread's cover alone), drawn as
		 * a check box rather than a second fill: two filled pills side by side
		 * read as one block cut by a seam.
		 */
		check?: boolean;
		onclick: () => void;
		children: Snippet;
	} = $props();
</script>

<!-- On is filled with the accent and off is dim: a tint beside bright grey read the wrong way round.
     12 px with light tracking: at 10 and 11 px the spaced capitals were the hardest words in the reader to read. -->
<button
	type="button"
	{disabled}
	aria-label={label}
	aria-pressed={active}
	title={label}
	class="inline-flex h-8 min-w-8 items-center justify-center gap-1.5 rounded-lg px-2 font-mono text-xs font-semibold uppercase tracking-[0.06em] transition-colors disabled:pointer-events-none disabled:opacity-40
		{check
		? active
			? 'text-foreground hover:bg-surface-hover'
			: 'text-foreground/55 hover:bg-surface-hover hover:text-foreground'
		: active
			? 'bg-accent text-accent-fg'
			: 'text-foreground/55 hover:bg-surface-hover hover:text-foreground'}"
	{onclick}
>
	{#if check}
		<span
			class="flex size-3.5 shrink-0 items-center justify-center rounded-[4px] border {active
				? 'border-accent bg-accent text-accent-fg'
				: 'border-current'}"
			aria-hidden="true"
		>
			{#if active}<Check class="size-3" strokeWidth={3} />{/if}
		</span>
	{/if}
	{@render children()}
</button>
