<script lang="ts">
	import type { CatalogItem, Enrichment } from '$lib/api/types';
	import { cardThumb } from '$lib/stores/frames';
	import { isReadable } from '$lib/reader/kinds';
	import { guessShape, recordShape, shapeOf, type Shape } from '$lib/stores/shapes.svelte';
	import PlaceholderPoster from './PlaceholderPoster.svelte';

	let {
		item,
		enrichment = null,
		class: className = '',
		priority = false,
		frame = undefined
	}: {
		item: CatalogItem;
		enrichment?: Enrichment | null;
		class?: string;
		/** Above-the-fold posters load eagerly; the rest are lazy. */
		priority?: boolean;
		/** The shape of the frame this is drawn in, shared by its grid or rail. */
		frame?: Shape;
	} = $props();

	let artworkFailed = $state(false);
	let thumbFailed = $state(false);

	// poster -> cover -> still frame (extracted and cached by the server).
	const artwork = $derived(!artworkFailed ? enrichment?.poster || enrichment?.cover || '' : '');
	// The same address the grids measure ahead of drawing (`settleFrame`), so this load is a cache hit.
	const thumb = $derived(!thumbFailed ? cardThumb(item) : '');
	const src = $derived(artwork || thumb);
	const alt = $derived(enrichment?.title || item.title);
	/*
	 * The fallback is a 16:9 still, not a poster. Cropping it to a 2:3 slot
	 * upscaled it 1.83x and threw away most of the picture, so a still is
	 * letterboxed inside the frame instead.
	 */
	// A comic or manga cover is a real portrait page, not a video still.
	const isStill = $derived(src === thumb && thumb !== '' && !isReadable(item.kind));
	// In a shared frame, a picture of the other shape is contained, never
	// cropped: a poster cut to 16:9 lost its title, a still cut to 2:3 most
	// of its picture.
	const contained = $derived(
		frame ? shapeOf(item.id, guessShape(item, artwork ? enrichment : null)) !== frame : isStill
	);

	function handleError(): void {
		if (artwork) artworkFailed = true;
		else thumbFailed = true;
	}
</script>

<div class={['relative overflow-hidden bg-surface', className].join(' ')}>
	{#if src}
		{#if contained}
			<!-- Fills the bars of a picture that does not match the frame. -->
			<img
				{src}
				alt=""
				aria-hidden="true"
				loading={priority ? 'eager' : 'lazy'}
				decoding="async"
				referrerpolicy="no-referrer"
				class="absolute inset-0 size-full scale-125 object-cover opacity-50 blur-2xl"
			/>
		{/if}
		<img
			{src}
			{alt}
			loading={priority ? 'eager' : 'lazy'}
			decoding="async"
			fetchpriority={priority ? 'high' : 'auto'}
			referrerpolicy="no-referrer"
			class={['relative size-full', contained ? 'object-contain' : 'object-cover'].join(' ')}
			onload={(e) => {
				const img = e.currentTarget as HTMLImageElement;
				recordShape(item.id, img.naturalWidth, img.naturalHeight);
			}}
			onerror={handleError}
		/>
	{:else}
		<PlaceholderPoster title={alt} seed={item.id} class="size-full" />
	{/if}
</div>
