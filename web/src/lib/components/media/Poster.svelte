<script lang="ts">
	import type { CatalogItem, Enrichment } from '$lib/api/types';
	import { api } from '$lib/api';
	import { session } from '$lib/auth/session.svelte';
	import PlaceholderPoster from './PlaceholderPoster.svelte';

	let {
		item,
		enrichment = null,
		class: className = '',
		priority = false
	}: {
		item: CatalogItem;
		enrichment?: Enrichment | null;
		class?: string;
		/** Above-the-fold posters load eagerly; the rest are lazy. */
		priority?: boolean;
	} = $props();

	let artworkFailed = $state(false);
	let thumbFailed = $state(false);

	// poster -> cover -> still frame (extracted and cached by the server).
	const artwork = $derived(!artworkFailed ? enrichment?.poster || enrichment?.cover || '' : '');
	const thumb = $derived(
		!thumbFailed ? api.thumbnail.url(item.id, session.token, { width: 320 }) : ''
	);
	const src = $derived(artwork || thumb);
	const alt = $derived(enrichment?.title || item.title);

	function handleError(): void {
		if (artwork) artworkFailed = true;
		else thumbFailed = true;
	}
</script>

<div class={['relative overflow-hidden bg-surface', className].join(' ')}>
	{#if src}
		<img
			{src}
			{alt}
			loading={priority ? 'eager' : 'lazy'}
			decoding="async"
			fetchpriority={priority ? 'high' : 'auto'}
			referrerpolicy="no-referrer"
			class="size-full object-cover"
			onerror={handleError}
		/>
	{:else}
		<PlaceholderPoster title={alt} seed={item.id} class="size-full" />
	{/if}
</div>
