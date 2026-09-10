<script lang="ts">
	import { onMount } from 'svelte';
	import type { Library } from '$lib/api/types';
	import LibraryBrowser from '$lib/components/media/LibraryBrowser.svelte';
	import { ensureLibraries } from '$lib/stores/media-cache.svelte';

	let libraries = $state<Library[]>([]);

	onMount(() => {
		void ensureLibraries()
			.then((libs) => (libraries = libs))
			.catch(() => {
				// The browser itself reports catalog failures; the
				// filter degrades to "all libraries" if this fails.
			});
	});
</script>

<svelte:head><title>Library — Lain</title></svelte:head>

<div class="space-y-6">
	<header>
		<h1 class="text-xl font-semibold tracking-tight text-foreground">Library</h1>
		<p class="mt-0.5 text-sm text-muted">Everything Lain has indexed.</p>
	</header>
	<LibraryBrowser {libraries} showLibraryFilter />
</div>
