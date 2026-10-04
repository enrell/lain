<script lang="ts">
	/*
	 * What a comic or manga archive says about itself (ComicInfo.xml),
	 * shown on the title page. Self-contained: it loads the reader view
	 * for one item and renders nothing when the archive is unreadable, so
	 * the page around it never waits on or fails because of it.
	 */
	import { api, type CatalogItem, type ReaderView } from '$lib/api';
	import Badge from '$lib/components/primitives/Badge.svelte';
	import Button from '$lib/components/primitives/Button.svelte';
	import { t } from '$lib/i18n';
	import { OfflineError, offlineStore, offlineSupported } from '$lib/offline/store';
	import { directionSource, infoRows } from '$lib/reader/info';
	import { toasts } from '$lib/stores/toasts.svelte';
	import { formatBytes } from '$lib/utilities/format';

	let { item }: { item: CatalogItem } = $props();
	const itemId = $derived(item.id);

	let view = $state<ReaderView | null>(null);
	// Offline copy in this browser (docs/slices/web-offline.md); `o` toggles.
	const canOffline = offlineSupported();
	let saved = $state(false);
	let saving = $state<{ done: number; total: number } | null>(null);

	$effect(() => {
		const id = itemId;
		saved = false;
		if (!canOffline) return;
		void offlineStore()
			.isSaved(id)
			.then((v) => {
				if (id === itemId) saved = v;
			})
			.catch(() => {});
	});

	async function toggleOffline(): Promise<void> {
		if (!canOffline || saving) return;
		const store = offlineStore();
		try {
			if (saved) {
				await store.remove(item.id);
				saved = false;
				toasts.success(t('offline.removedToast', { name: item.title }));
				return;
			}
			saving = { done: 0, total: view?.pages.length ?? 0 };
			await store.save(item, (done, total) => (saving = { done, total }));
			saved = true;
			toasts.success(t('offline.savedToast', { name: item.title }));
		} catch (err) {
			if (err instanceof OfflineError && err.code === 'quota') {
				toasts.error(t('offline.quota', { need: formatBytes(err.need), budget: formatBytes(err.budget) || '0 B' }));
			} else if (err instanceof OfflineError && err.code === 'unsupported') {
				toasts.error(t('offline.unsupported'));
			} else {
				toasts.error(t('offline.failed'));
			}
		} finally {
			saving = null;
		}
	}

	function onKey(e: KeyboardEvent): void {
		if (e.key !== 'o' || e.ctrlKey || e.metaKey || e.altKey || e.defaultPrevented) return;
		const el = e.target as HTMLElement | null;
		if (el && (['INPUT', 'TEXTAREA', 'SELECT'].includes(el.tagName) || el.isContentEditable)) return;
		if (document.querySelector('[role="dialog"]')) return;
		e.preventDefault();
		void toggleOffline();
	}

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

<svelte:window onkeydown={onKey} />

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
		{#if canOffline}
			<Button size="sm" variant="secondary" loading={!!saving} onclick={() => void toggleOffline()}>
				{saving
					? t('offline.saving', { done: saving.done, total: saving.total })
					: saved
						? t('offline.remove')
						: t('offline.save')}
				<kbd class="ms-1 font-mono text-[10px] opacity-70">o</kbd>
			</Button>
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
