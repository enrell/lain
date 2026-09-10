<script lang="ts">
	import type { CatalogItem, Enrichment } from '$lib/api/types';
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

	let failed = $state(false);
	const src = $derived(!failed && enrichment?.poster ? enrichment.poster : '');
	const alt = $derived(enrichment?.title || item.title);
</script>

<div class={['relative overflow-hidden bg-surface', className].join(' ')}>
	{#if src}
		<img
			src={src}
			{alt}
			loading={priority ? 'eager' : 'lazy'}
			decoding="async"
			fetchpriority={priority ? 'high' : 'auto'}
			referrerpolicy="no-referrer"
			class="size-full object-cover"
			onerror={() => (failed = true)}
		/>
	{:else}
		<PlaceholderPoster title={alt} seed={item.id} class="size-full" />
	{/if}
</div>
