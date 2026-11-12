<script lang="ts">
	import { onMount } from 'svelte';
	import ListChecks from '@lucide/svelte/icons/list-checks';
	import { api, type ListEntry } from '$lib/api';
	import EmptyState from '$lib/components/primitives/EmptyState.svelte';
	import ErrorState from '$lib/components/primitives/ErrorState.svelte';
	import Skeleton from '$lib/components/primitives/Skeleton.svelte';
	import ListEntryCard from '$lib/components/media/ListEntryCard.svelte';
	import { errorMessage } from '$lib/utilities/errors';
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

	async function load(): Promise<void> {
		loadError = '';
		try {
			const res = await api.list.entries({
				type: typeFilter || undefined,
				status: statusFilter || undefined
			});
			entries = res.entries;
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
</script>

<svelte:head><title>Tracking — Lain</title></svelte:head>

<div class="space-y-6">
	<header>
		<h1 class="text-xl font-semibold tracking-tight text-foreground">{t('nav.list')}</h1>
		<p class="mt-0.5 text-sm text-muted">{t('tracking.subtitle')}</p>
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
		<div
			class="grid grid-cols-2 gap-x-3 gap-y-5 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 xl:grid-cols-6"
		>
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
	{:else if entries.length === 0}
		<EmptyState
			title={t('tracking.emptyTitle')}
			description={t('tracking.emptyBody')}
		>
			{#snippet icon()}<ListChecks class="size-6 text-muted" />{/snippet}
			<a
				href="/settings/connections"
				class="inline-flex h-10 items-center rounded-md bg-accent px-4 text-sm font-medium text-accent-fg hover:bg-accent-hover"
				>{t('tracking.openConnections')}</a
			>
		</EmptyState>
	{:else}
		<div
			class="grid grid-cols-2 gap-x-3 gap-y-5 sm:grid-cols-3 md:grid-cols-4 lg:grid-cols-5 xl:grid-cols-6"
		>
			{#each entries as entry, i (entry.id)}
				<ListEntryCard {entry} priority={i < 6} />
			{/each}
		</div>
	{/if}
</div>
