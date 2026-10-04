<script lang="ts">
	/*
	 * What a comic or manga archive says about itself (ComicInfo.xml),
	 * shown on the title page. Self-contained: it loads the reader view
	 * for one item and renders nothing when the archive is unreadable, so
	 * the page around it never waits on or fails because of it.
	 */
	import { api, type ReaderView } from '$lib/api';
	import Badge from '$lib/components/primitives/Badge.svelte';
	import { t } from '$lib/i18n';
	import { directionSource, infoRows } from '$lib/reader/info';

	let { itemId }: { itemId: string } = $props();

	let view = $state<ReaderView | null>(null);

	$effect(() => {
		const id = itemId;
		let live = true;
		view = null;
		api.reader
			.pages(id)
			.then((v) => {
				if (live) view = v;
			})
			.catch(() => {
				// Missing file or no extractor: the page already says so.
			});
		return () => {
			live = false;
		};
	});

	const rows = $derived(infoRows(view?.info));
</script>

{#if view}
	<section class="max-w-3xl space-y-3" aria-label={t('reader.info.label')}>
		<h2 class="font-mono text-[10px] font-semibold uppercase tracking-[0.26em] text-muted">
			{t('reader.info.label')}
		</h2>
		{#if rows.length}
			<dl class="grid gap-x-8 gap-y-1.5 text-sm sm:grid-cols-[max-content_minmax(0,1fr)]">
				{#each rows as row (row.field)}
					<dt class="font-mono text-[10px] uppercase tracking-[0.14em] text-muted sm:pt-0.5">
						{t(`reader.info.field.${row.field}`)}
					</dt>
					<dd class="text-foreground">{row.value}</dd>
				{/each}
			</dl>
		{/if}
		<p class="text-xs text-muted">
			{t(`reader.info.direction.${view.direction}`)} ·
			{t(`reader.info.source.${directionSource(view.info)}`)} ·
			{t('reader.info.pages', { count: view.pages.length })}
		</p>
		{#if view.info?.genres?.length}
			<div class="flex flex-wrap gap-1.5">
				{#each view.info.genres as genre (genre)}<Badge>{genre}</Badge>{/each}
			</div>
		{/if}
		{#if view.info?.summary}
			<p class="text-sm leading-relaxed text-muted">{view.info.summary}</p>
		{/if}
	</section>
{/if}
