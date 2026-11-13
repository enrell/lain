<script lang="ts">
	// The site's live demo has no media behind it. Where the player would be,
	// it offers a well-known video from its publisher's own channel, loaded
	// only when asked for (nothing is requested from another site before).
	import Play from '@lucide/svelte/icons/play';

	let { title }: { title: string } = $props();
	let playing = $state(false);
	const VIDEO = 'dQw4w9WgXcQ';
</script>

<div class="relative aspect-video w-full overflow-hidden bg-black">
	{#if playing}
		<iframe
			class="absolute inset-0 size-full"
			src="https://www.youtube-nocookie.com/embed/{VIDEO}?autoplay=1&rel=0"
			title="A video the demo plays instead of {title}"
			allow="autoplay; encrypted-media; picture-in-picture; fullscreen"
			referrerpolicy="strict-origin-when-cross-origin"
		></iframe>
	{:else}
		<div class="absolute inset-0 flex flex-col items-center justify-center gap-5 px-6 text-center">
			<p class="font-mono text-[10px] uppercase tracking-[0.28em] text-accent">Live demo</p>
			<p class="max-w-md text-lg font-semibold text-foreground">This demo has no files behind it, so {title} cannot play here.</p>
			<p class="max-w-md text-sm text-muted">On your own server this is the player: direct play, subtitles, seek previews. Here, Play brings something else.</p>
			<button
				type="button"
				class="inline-flex h-12 items-center gap-2 rounded-lg bg-accent px-6 text-sm font-semibold text-accent-fg hover:bg-accent-hover"
				onclick={() => (playing = true)}
			>
				<Play class="size-4" fill="currentColor" /> Play anyway
			</button>
		</div>
	{/if}
</div>
