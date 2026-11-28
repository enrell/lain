<script lang="ts">
	/*
	 * What a book says about itself (D-306, D-308): authors, publisher,
	 * language, date, format and length, from its own file, and the copy
	 * for offline reading (`o`). Self-contained like the comic panel: it
	 * renders nothing when the book cannot be read.
	 */
	import { api, type BookManifest, type CatalogItem } from '$lib/api';
	import Badge from '$lib/components/primitives/Badge.svelte';
	import Button from '$lib/components/primitives/Button.svelte';
	import { t } from '$lib/i18n';
	import { OfflineError, offlineStore, offlineSupported } from '$lib/offline/store';
	import { toasts } from '$lib/stores/toasts.svelte';
	import { formatBytes } from '$lib/utilities/format';

	let { item, summary = true }: { item: CatalogItem; summary?: boolean } = $props();
	const itemId = $derived(item.id);
	let book = $state<BookManifest | null>(null);
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
			saving = { done: 0, total: book?.pages ?? 0 };
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
		book = null;
		api.reader
			.book(id)
			.then((b) => {
				if (live) book = b;
			})
			.catch(() => {
				// Unreadable or protected: the reader says why.
			});
		return () => {
			live = false;
		};
	});

	const rows = $derived(
		book
			? ([
					['authors', book.authors?.join(', ') ?? ''],
					['publisher', book.publisher ?? ''],
					['language', book.language ?? ''],
					['published', book.published ?? ''],
					['format', book.format.toUpperCase()]
				] as const).filter(([, v]) => v !== '')
			: []
	);
</script>

<svelte:window onkeydown={onKey} />

{#if book}
	<section class="max-w-3xl space-y-3" aria-label={t('book.info.label')}>
		<h2 class="font-mono text-[10px] font-semibold uppercase tracking-[0.26em] text-muted">{t('book.info.label')}</h2>
		{#if rows.length}
			<dl class="grid gap-x-8 gap-y-1.5 text-sm sm:grid-cols-[max-content_minmax(0,1fr)]">
				{#each rows as [field, value] (field)}
					<dt class="font-mono text-[10px] uppercase tracking-[0.14em] text-muted sm:pt-0.5">{t(`book.info.field.${field}`)}</dt>
					<dd class="text-foreground">{value}</dd>
				{/each}
			</dl>
		{/if}
		{#if canOffline}
			<Button size="sm" variant="tonal" loading={!!saving} onclick={() => void toggleOffline()}>
				{saving ? t('offline.saving', { done: saving.done, total: saving.total }) : saved ? t('offline.remove') : t('offline.save')}
				<kbd class="ms-1 font-mono text-[10px] opacity-70">o</kbd>
			</Button>
		{/if}
		<p class="text-xs text-muted">
			{book.layout === 'pages' ? t('book.info.realPages', { count: book.pages }) : t('book.info.pages', { count: book.pages })}
			{#if book.warnings?.includes('fonts-missing')} · {t('book.info.fontsMissing')}{/if}
		</p>
		{#if book.subjects?.length}
			<div class="flex flex-wrap gap-1.5">
				{#each book.subjects as s (s)}<Badge>{s}</Badge>{/each}
			</div>
		{/if}
		{#if summary && book.description}
			<p class="text-sm leading-relaxed text-muted">{book.description}</p>
		{/if}
	</section>
{/if}
