<script lang="ts">
	import { onMount } from 'svelte';
	import ListChecks from '@lucide/svelte/icons/list-checks';
	import { api, type ListEntry } from '$lib/api';
	import EmptyState from '$lib/components/primitives/EmptyState.svelte';
	import ErrorState from '$lib/components/primitives/ErrorState.svelte';
	import Skeleton from '$lib/components/primitives/Skeleton.svelte';
	import ListEntryCard from '$lib/components/media/ListEntryCard.svelte';
	import { errorMessage } from '$lib/utilities/errors';

	// The unified tracking list (D-078): everything a linked account
	// imported, browsable by media type and status.
	const types = [
		{ id: '', label: 'All' },
		{ id: 'anime', label: 'Anime' },
		{ id: 'manga', label: 'Manga' },
		{ id: 'movie', label: 'Movies' },
		{ id: 'series', label: 'Series' },
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

<svelte:head><title>My list — Lain</title></svelte:head>

<div class="space-y-6">
	<header>
		<h1 class="text-xl font-semibold tracking-tight text-foreground">My list</h1>
		<p class="mt-0.5 text-sm text-muted">
			Everything your connected accounts track — manga, comics, anime, movies and series in
			one place.
		</p>
	</header>

	<div class="flex flex-wrap items-center gap-2" role="group" aria-label="Media type">
		{#each types as t (t.id)}
			<button
				type="button"
				aria-pressed={typeFilter === t.id}
				class={[
					'rounded-full border px-3 py-1.5 text-xs font-medium transition-colors',
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
			class="rounded-full border border-line bg-surface/60 px-3 py-1.5 text-xs font-medium text-muted hover:text-foreground"
			value={statusFilter}
			aria-label="Status"
			onchange={(e) => pickStatus(e.currentTarget.value)}
		>
			{#each statuses as s (s.id)}
				<option value={s.id}>{s.label}</option>
			{/each}
		</select>
	</div>

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
	{:else if entries.length === 0}
		<EmptyState
			title="Nothing on your list yet"
			description="Connect an account in Settings → Account to import what you track. AniList is supported today."
		>
			{#snippet icon()}<ListChecks class="size-6 text-muted" />{/snippet}
			<a
				href="/settings"
				class="rounded-md border border-line bg-surface px-3 py-1.5 text-xs font-medium text-foreground"
				>Open account settings</a
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
