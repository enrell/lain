<script lang="ts">
	import type { ListEntry } from '$lib/api/types';
	import PlaceholderPoster from './PlaceholderPoster.svelte';
	import ProgressBar from './ProgressBar.svelte';

	let {
		entry,
		priority = false
	}: {
		entry: ListEntry;
		priority?: boolean;
	} = $props();

	let coverFailed = $state(false);
	const cover = $derived(!coverFailed ? entry.cover || '' : '');

	const statusLabel: Record<string, string> = {
		current: 'Current',
		planning: 'Planning',
		completed: 'Completed',
		paused: 'Paused',
		dropped: 'Dropped',
		repeating: 'Repeating'
	};
	const status = $derived(statusLabel[entry.status] ?? entry.status);
	const progress = $derived(
		entry.progress_total ? `${entry.progress}/${entry.progress_total}` : `${entry.progress}`
	);
	const ratio = $derived(
		entry.progress_total ? Math.min(1, entry.progress / entry.progress_total) : 0
	);
	const meta = $derived(
		[entry.media_type, status, `progress ${progress}`].filter(Boolean).join(' · ')
	);
	const accessibleName = $derived(`${entry.title}, ${meta}`);
</script>

<div
	class="group block rounded-card transition-transform duration-300 ease-out hover:-translate-y-1"
	aria-label={accessibleName}
>
	<div
		class="relative overflow-hidden rounded-xl bg-surface shadow-[0_18px_45px_rgba(0,0,0,0.16)] ring-1 ring-white/5 transition duration-300 group-hover:ring-white/15"
	>
		<div class="aspect-[2/3]">
			{#if cover}
				<img
					src={cover}
					alt={entry.title}
					loading={priority ? 'eager' : 'lazy'}
					decoding="async"
					referrerpolicy="no-referrer"
					class="size-full object-cover transition-transform duration-500 ease-out group-hover:scale-[1.035]"
					onerror={() => (coverFailed = true)}
				/>
			{:else}
				<PlaceholderPoster title={entry.title} seed={entry.id} class="size-full" />
			{/if}
		</div>
		<span
			class="absolute left-2 top-2 rounded-full bg-black/65 px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wider text-foreground backdrop-blur"
		>
			{entry.platform}
		</span>
		{#if ratio > 0}
			<div class="absolute inset-x-0 bottom-0 p-1.5">
				<ProgressBar {ratio} label="Progress" class="bg-black/50" />
			</div>
		{/if}
	</div>
	<div class="mt-3 min-w-0">
		<p class="line-clamp-2 text-sm font-semibold leading-snug tracking-[-0.015em] text-foreground">
			{entry.title}
		</p>
		<p class="mt-1 truncate text-xs text-muted">
			{entry.media_type} · {status} · {progress}{#if entry.score} · ★ {entry.score}{/if}
		</p>
	</div>
</div>
