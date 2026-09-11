<script lang="ts">
	import ChevronDown from '@lucide/svelte/icons/chevron-down';
	import type { Progress } from '$lib/api/types';
	import type { SeriesGroup } from '$lib/utilities/grouping';
	import MediaGrid from './MediaGrid.svelte';

	/** One multi-episode show: header plus its episodes in watch order.
	 * Single-file titles never reach this component; the browser renders
	 * them together in one shared grid. The header is a plain div (not a
	 * link) so episode cards stay the only `/item/` targets here. */
	let {
		group,
		progressMap = null
	}: {
		group: SeriesGroup;
		progressMap?: Map<string, Progress> | null;
	} = $props();

	let expanded = $state(true);

	const seasons = $derived([...new Set(group.items.map((i) => i.season).filter((s) => s > 0))]);

	const detail = $derived.by(() => {
		const parts: string[] = [`${group.count} episodes`];
		if (seasons.length === 1) parts.push(`Season ${seasons[0]}`);
		else if (seasons.length > 1)
			parts.push(`Seasons ${Math.min(...seasons)}–${Math.max(...seasons)}`);
		if (group.year > 0) parts.push(String(group.year));
		return parts.join(' · ');
	});
</script>

<section aria-label={group.title} class="space-y-3">
	<div class="flex flex-wrap items-baseline justify-between gap-x-4 gap-y-1">
		<div class="min-w-0">
			<h2 class="truncate text-base font-semibold tracking-tight text-foreground">
				{group.title}
			</h2>
			<p class="mt-0.5 text-xs text-muted">{detail}</p>
		</div>
		<button
			type="button"
			onclick={() => (expanded = !expanded)}
			aria-expanded={expanded}
			class="flex shrink-0 items-center gap-1 rounded-md px-2 py-1 text-xs font-medium text-muted transition-colors hover:bg-surface-hover hover:text-foreground"
		>
			{expanded ? 'Collapse' : `Expand (${group.count})`}
			<ChevronDown
				class={['size-3.5 transition-transform', expanded ? 'rotate-180' : ''].join(' ')}
				aria-hidden="true"
			/>
		</button>
	</div>
	{#if expanded}
		<MediaGrid items={group.items} {progressMap} />
	{/if}
</section>
