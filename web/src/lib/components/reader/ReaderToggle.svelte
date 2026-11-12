<script lang="ts">
	import type { Snippet } from 'svelte';

	let {
		label,
		active = false,
		disabled = false,
		onclick,
		children
	}: {
		label: string;
		active?: boolean;
		disabled?: boolean;
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
		{active ? 'bg-accent text-accent-fg' : 'text-foreground/55 hover:bg-surface-hover hover:text-foreground'}"
	{onclick}
>
	{@render children()}
</button>
