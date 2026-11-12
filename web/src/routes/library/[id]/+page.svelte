<script lang="ts">
	import { onMount } from 'svelte';
	import { page } from '$app/state';
	import LibraryOff from '@lucide/svelte/icons/library';
	import type { Library } from '$lib/api/types';
	import EmptyState from '$lib/components/primitives/EmptyState.svelte';
	import LibraryBrowser from '$lib/components/media/LibraryBrowser.svelte';
	import { ensureLibraries } from '$lib/stores/media-cache.svelte';

	const id = $derived(page.params.id ?? '');
	let libraries = $state<Library[]>([]);
	let loaded = $state(false);

	onMount(() => {
		void ensureLibraries()
			.then((libs) => (libraries = libs))
			.catch(() => {})
			.finally(() => (loaded = true));
	});

	const library = $derived(libraries.find((lib) => lib.id === id));
</script>

<svelte:head><title>{library?.name ?? 'Library'} — Lain</title></svelte:head>

<div class="space-y-6">
	{#if !loaded}
		<div class="h-10 w-56 animate-pulse rounded-md bg-surface-active/70"></div>
	{:else if !library}
		<EmptyState
			title="Library not found"
			description="It may have been removed. Other libraries are still available."
		>
			{#snippet icon()}<LibraryOff class="size-6 text-muted" />{/snippet}
		</EmptyState>
	{:else}
		<LibraryBrowser {libraries} libraryId={library.id} name={library.name} backHref="/library">
			{#snippet header()}
				<header>
					<a href="/library" class="font-mono text-[11px] uppercase tracking-[0.14em] text-muted hover:text-foreground" title="Back (Backspace)">‹ Library</a>
					<h1 class="mt-1 text-xl font-semibold tracking-tight text-foreground">{library.name}</h1>
				</header>
			{/snippet}
		</LibraryBrowser>
	{/if}
</div>
