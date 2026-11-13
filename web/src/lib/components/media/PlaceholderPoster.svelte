<script lang="ts">
	/*
	 * The picture a title shows when it has none (D-287): its name lettered
	 * in large tight type on the theme's surface, under the accent's short
	 * bar, with the house's small mono label for the part. The server draws
	 * the same for a thumbnail it cannot make (an SVG), so a card looks the
	 * same whichever side gave up. Never an empty box: a library still
	 * reads as a designed grid when its disk is away or its art is missing.
	 */
	let {
		title,
		seed = '',
		label = '',
		wide = false,
		class: className = ''
	}: {
		title: string;
		/** Kept for callers; the look no longer varies by item. */
		seed?: string;
		/** A small mono line under the name (S01E03, a year). */
		label?: string;
		/** A 16:9 slot, where the name is set smaller than on a 2:3 cover. */
		wide?: boolean;
		class?: string;
	} = $props();
	

	const name = $derived(title.trim() || '?');
</script>

<div
	class={['relative flex flex-col justify-end overflow-hidden bg-gradient-to-br from-surface-active to-background', className].join(' ')}
	style="container-type: inline-size;"
	aria-hidden="true"
>
	<span class="absolute left-[7cqi] top-[7cqi] h-[max(2px,0.8cqi)] w-[8cqi] rounded-full bg-accent"></span>
	<div class="relative p-[7cqi]">
		<p
			class="line-clamp-3 break-words font-bold leading-[1.05] tracking-[-0.02em] text-foreground"
			style:font-size={wide ? 'max(0.75rem, 7.3cqi)' : 'max(0.75rem, 12cqi)'}
		>
			{name}
		</p>
		{#if label}
			<p class="mt-[3cqi] truncate font-mono font-semibold uppercase tracking-[0.18em] text-muted" style:font-size={wide ? 'max(0.5625rem, 2.8cqi)' : 'max(0.5625rem, 4.5cqi)'}>
				{label}
			</p>
		{/if}
	</div>
</div>
