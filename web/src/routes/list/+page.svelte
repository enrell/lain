<script lang="ts">
	import { onMount } from 'svelte';
	import ListChecks from '@lucide/svelte/icons/list-checks';
	import { goto } from '$app/navigation';
	import { api, type LinkedAccountView, type ListEntry } from '$lib/api';
	import FirstRun from '$lib/components/media/FirstRun.svelte';
	import { framedGrid } from '$lib/components/media/grid';
	import Button from '$lib/components/primitives/Button.svelte';
	import EmptyState from '$lib/components/primitives/EmptyState.svelte';
	import LinkButton from '$lib/components/primitives/LinkButton.svelte';
	import ErrorState from '$lib/components/primitives/ErrorState.svelte';
	import Skeleton from '$lib/components/primitives/Skeleton.svelte';
	import ListEntryCard from '$lib/components/media/ListEntryCard.svelte';
	import { toasts } from '$lib/stores/toasts.svelte';
	import { errorMessage } from '$lib/utilities/errors';
	import { isTypingTarget } from '$lib/utilities/guards';
	import { usePageKeys } from '$lib/stores/page-keys.svelte';
	import { t } from '$lib/i18n';

	// The unified tracking list (D-078): everything a linked account
	// imported, browsable by media type and status.
	const types = [
		{ id: '', label: 'All' },
		{ id: 'anime', label: 'Anime' },
		{ id: 'manga', label: 'Manga' },
		// The libraries' words for the same things.
		{ id: 'movie', label: t('tracking.films') },
		{ id: 'series', label: t('tracking.shows') },
		{ id: 'comic', label: 'Comics' }
	];
	const statuses = [
		{ id: '', label: 'Any status' },
		{ id: 'current', label: 'Current' },
		{ id: 'planning', label: 'Planning' },
		{ id: 'completed', label: 'Completed' },
		{ id: 'paused', label: 'Paused' },
		{ id: 'dropped', label: 'Dropped' },
		{ id: 'repeating', label: 'Repeating' }
	];

	let entries = $state<ListEntry[]>([]);
	let ready = $state(false);
	let loadError = $state('');
	let typeFilter = $state('');
	let statusFilter = $state('');
	// The accounts linked: an empty list with one linked asks for a sync, not a connection.
	let links = $state<LinkedAccountView[]>([]);
	let syncing = $state(false);

	async function load(): Promise<void> {
		loadError = '';
		try {
			const [res, linked] = await Promise.all([
				api.list.entries({
					type: typeFilter || undefined,
					status: statusFilter || undefined
				}),
				api.links.list().then((r) => r.links).catch(() => [] as LinkedAccountView[])
			]);
			entries = res.entries;
			links = linked;
		} catch (err) {
			loadError = errorMessage(err, 'Could not load your list.');
		} finally {
			ready = true;
		}
	}

	// The filters narrow a list; over nothing tracked at all they had nothing to narrow.
	const filtered = $derived(typeFilter !== '' || statusFilter !== '');
	const showFilters = $derived(ready && !loadError && (entries.length > 0 || filtered));

	function showEverything(): void {
		typeFilter = '';
		statusFilter = '';
		void load();
	}

	function pickType(id: string): void {
		typeFilter = id;
		void load();
	}
	function pickStatus(id: string): void {
		statusFilter = id;
		void load();
	}

	onMount(() => void load());

	// Nothing tracked and no filter set: the page says what it is for.
	const firstRun = $derived(ready && !loadError && entries.length === 0 && !filtered);
	const link = $derived(links[0]);

	async function sync(): Promise<void> {
		if (!link || syncing) return;
		syncing = true;
		try {
			const res = await api.links.sync(link.platform);
			toasts.success(t('tracking.linkedEmpty.synced', { count: res.stats.upserted }));
			await load();
		} catch (err) {
			toasts.error(errorMessage(err, t('tracking.linkedEmpty.syncFailed')));
		} finally {
			syncing = false;
		}
	}

	// The first step's key: C connects an account, S syncs the one linked.
	usePageKeys(() => (firstRun ? [link ? { keys: ['S'], label: t('pageKeys.sync') } : { keys: ['C'], label: t('pageKeys.connect') }] : []));

	function onKey(e: KeyboardEvent): void {
		if (!firstRun || e.ctrlKey || e.metaKey || e.altKey || e.defaultPrevented) return;
		if (isTypingTarget(e.target) || document.querySelector('[role="dialog"]')) return;
		if (e.key === 'c' && !link) {
			e.preventDefault();
			void goto('/settings/connections');
		} else if (e.key === 's' && link) {
			e.preventDefault();
			void sync();
		}
	}
</script>

<svelte:head><title>Tracking — Lain</title></svelte:head>
<svelte:window onkeydown={onKey} />

<div class="space-y-6">
	<header>
		<h1 class="page-title">{t('nav.list')}</h1>
		<!-- The first-run block below says what the page is for, at length. -->
		{#if !firstRun}<p class="mt-0.5 text-sm text-muted">{t('tracking.subtitle')}</p>{/if}
	</header>

	{#if showFilters}
	<div class="flex flex-wrap items-center gap-2" role="group" aria-label="Media type">
		{#each types as t (t.id)}
			<button
				type="button"
				aria-pressed={typeFilter === t.id}
				class={[
					// The size of every other chip (recent searches, browse links): at 12 px
					// these were the one set of small chips in the product.
					'inline-flex min-h-9 items-center rounded-full border px-3.5 text-sm font-medium transition-colors',
					typeFilter === t.id
						? 'border-accent bg-accent/15 text-foreground'
						: 'border-line bg-surface/60 text-muted hover:text-foreground'
				].join(' ')}
				onclick={() => pickType(t.id)}
			>
				{t.label}
			</button>
		{/each}
		<span class="mx-1 hidden h-4 w-px bg-line sm:inline" aria-hidden="true"></span>
		<select
			class="h-9 rounded-full border border-line bg-surface/60 ps-3.5 text-sm font-medium leading-none text-muted hover:text-foreground"
			value={statusFilter}
			aria-label="Status"
			onchange={(e) => pickStatus(e.currentTarget.value)}
		>
			{#each statuses as s (s.id)}
				<option value={s.id}>{s.label}</option>
			{/each}
		</select>
	</div>
	{/if}

	{#if loadError}
		<ErrorState message={loadError} retry={() => void load()} />
	{:else if !ready}
		<div class={framedGrid('portrait')}>
			{#each Array(12) as _}
				<Skeleton class="aspect-[2/3] w-full rounded-xl" />
			{/each}
		</div>
	{:else if entries.length === 0 && filtered}
		<!-- Something is tracked, just not of this kind: "connect an account" would mislead. -->
		<EmptyState title={t('tracking.noMatchTitle')} description={t('tracking.noMatchBody')}>
			{#snippet icon()}<ListChecks class="size-6 text-muted" />{/snippet}
			<button
				type="button"
				class="inline-flex h-10 items-center rounded-md border border-line px-4 text-sm font-medium text-foreground hover:bg-surface-hover"
				onclick={showEverything}>{t('tracking.showEverything')}</button
			>
		</EmptyState>
	{:else if entries.length === 0 && link}
		<FirstRun
			eyebrow={t('tracking.linkedEmpty.eyebrow')}
			title={t('tracking.linkedEmpty.title')}
			body={t('tracking.linkedEmpty.body', { name: link.remote_username || link.platform })}
		>
			<Button size="rowMain" loading={syncing} onclick={() => void sync()}>{t('tracking.linkedEmpty.sync')} <kbd>S</kbd></Button>
			<LinkButton href="/settings/connections" variant="tonal" size="row">{t('tracking.linkedEmpty.connections')}</LinkButton>
		</FirstRun>
	{:else if entries.length === 0}
		<FirstRun
			eyebrow={t('tracking.firstRun.eyebrow')}
			title={t('tracking.firstRun.title')}
			body={t('tracking.firstRun.body')}
			steps={[
				{ label: t('tracking.firstRun.stepConnect'), text: t('tracking.firstRun.stepConnectText') },
				{ label: t('tracking.firstRun.stepImport'), text: t('tracking.firstRun.stepImportText') },
				{ label: t('tracking.firstRun.stepWatch'), text: t('tracking.firstRun.stepWatchText') }
			]}
		>
			<LinkButton href="/settings/connections" size="rowMain">{t('tracking.firstRun.connect')} <kbd>C</kbd></LinkButton>
			<LinkButton href="/library" variant="tonal" size="row">{t('tracking.firstRun.watch')}</LinkButton>
		</FirstRun>
	{:else}
		<div class={framedGrid('portrait')}>
			{#each entries as entry, i (entry.id)}
				<ListEntryCard {entry} priority={i < 6} />
			{/each}
		</div>
	{/if}
</div>
