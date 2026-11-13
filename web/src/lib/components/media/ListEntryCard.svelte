<script lang="ts">
	import Search from '@lucide/svelte/icons/search';
	import type { ListEntry } from '$lib/api/types';
	import PlaceholderPoster from './PlaceholderPoster.svelte';
	import CardProgress from './CardProgress.svelte';
	import { t } from '$lib/i18n';
	import { formatRelative } from '$lib/utilities/format';
	import { acquireHref, isReadingMedia, platformName, progressText, statusLabel } from '$lib/tracking/status';

	/*
	 * One entry of a linked list (D-288). An entry the library holds is a
	 * link to its title; one it lacks says so by having no link, and a
	 * planned one offers an administrator Acquire's search for it. Under
	 * the name: the list, the progress and the score, then when lain last
	 * sent it (or why it could not).
	 */
	let {
		entry,
		priority = false,
		canAcquire = false
	}: {
		entry: ListEntry;
		priority?: boolean;
		/** Whether the viewer may search releases (an administrator). */
		canAcquire?: boolean;
	} = $props();

	let coverFailed = $state(false);
	const cover = $derived(!coverFailed ? entry.cover || '' : '');
	// A wide cover is shown whole over a blurred copy, as a still is in a
	// poster frame elsewhere; cropping it to 2:3 cut its title off.
	let wide = $state(false);

	const reading = $derived(isReadingMedia(entry.media_type));
	const status = $derived(statusLabel(entry.status, reading));
	const progress = $derived(progressText(entry));
	const ratio = $derived(entry.progress_total ? Math.min(1, entry.progress / entry.progress_total) : 0);
	const score = $derived(entry.score ? t('tracking.score', { score: entry.score }) : '');
	const meta = $derived([status, progress, score].filter(Boolean).join(' · '));
	const inLibrary = $derived(!!entry.item_id);
	const findable = $derived(!inLibrary && canAcquire && entry.status === 'planning');
	const sync = $derived(
		entry.push_error
			? t('tracking.notSent', { error: entry.push_error })
			: entry.pushed_at
				? t('tracking.sentWhen', { when: formatRelative(entry.pushed_at) })
				: ''
	);
</script>

{#snippet art()}
	<div
		class="relative overflow-hidden rounded-xl bg-surface shadow-[0_18px_45px_rgba(0,0,0,0.16)] ring-1 ring-white/5 transition duration-300 group-hover:ring-white/15 group-focus-visible:ring-2 group-focus-visible:ring-accent"
	>
		<div class="relative aspect-[2/3]">
			{#if cover}
				{#if wide}
					<img src={cover} alt="" aria-hidden="true" decoding="async" referrerpolicy="no-referrer" class="absolute inset-0 size-full scale-125 object-cover opacity-50 blur-2xl" />
				{/if}
				<img
					src={cover}
					alt=""
					loading={priority ? 'eager' : 'lazy'}
					decoding="async"
					referrerpolicy="no-referrer"
					class={['relative size-full transition-transform duration-500 ease-out group-hover:scale-[1.035]', wide ? 'object-contain' : 'object-cover', inLibrary ? '' : 'opacity-80'].join(' ')}
					onload={(e) => {
						const img = e.currentTarget as HTMLImageElement;
						wide = img.naturalWidth > img.naturalHeight * 1.15;
					}}
					onerror={() => (coverFailed = true)}
				/>
			{:else}
				<PlaceholderPoster title={entry.title} seed={entry.id} class="size-full" />
			{/if}
		</div>
		<span class="absolute start-2 top-2 rounded-full bg-black/65 px-2 py-0.5 font-mono text-[10px] uppercase tracking-[0.14em] text-foreground backdrop-blur">
			{platformName(entry.platform)}
		</span>
		{#if inLibrary}
			<span class="absolute end-2 top-2 inline-flex items-center gap-1 rounded-full bg-accent/90 px-2 py-0.5 font-mono text-[10px] uppercase tracking-[0.14em] text-accent-fg backdrop-blur">
				{t('tracking.inLibrary')}
			</span>
		{/if}
		{#if ratio > 0}
			<CardProgress {ratio} label={t('tracking.entryProgress')} />
		{/if}
	</div>
	<div class="mt-3 min-w-0">
		<p class="line-clamp-2 text-sm font-semibold leading-snug tracking-[-0.015em] text-foreground">{entry.title}</p>
		<p class="mt-1 truncate text-xs text-muted">{meta}</p>
		{#if sync}
			<p class={['mt-0.5 truncate font-mono text-[10px] uppercase tracking-[0.14em]', entry.push_error ? 'text-danger' : 'text-muted/80'].join(' ')}>{sync}</p>
		{/if}
	</div>
{/snippet}

{#if inLibrary}
	<a
		href={`/item/${encodeURIComponent(entry.item_id ?? '')}`}
		class="group block rounded-card outline-none transition-transform duration-300 ease-out hover:-translate-y-1"
		aria-label={t('tracking.openTitle', { title: `${entry.title}, ${meta}` })}
		data-list-card
	>
		{@render art()}
	</a>
{:else}
	<div class="group block rounded-card" data-list-card>
		{@render art()}
		{#if findable}
			<a
				href={acquireHref(entry)}
				class="mt-2 inline-flex min-h-9 items-center gap-1.5 rounded-md border border-line px-2.5 text-xs font-medium text-foreground hover:bg-surface-hover focus-visible:ring-2 focus-visible:ring-accent"
				title={t('tracking.findReleasesHint', { title: entry.title })}
			>
				<Search class="size-3.5" aria-hidden="true" />{t('tracking.findReleases')}
			</a>
		{/if}
	</div>
{/if}
