<script lang="ts">
	import { onMount } from 'svelte';
	import LinkButton from '$lib/components/primitives/LinkButton.svelte';
	import { externalPlayerUrl } from '$lib/player/external-player';

	let { itemId, onOpen }: { itemId: string; onOpen?: () => void } = $props();
	let origin = $state('');
	onMount(() => { origin = window.location.origin; });

	function href(player: 'mpv' | 'vlc'): string { return externalPlayerUrl(origin, itemId, player); }
</script>

{#if origin}
	<div class="space-y-2">
		<div class="flex flex-wrap items-center gap-2">
			<LinkButton href={href('mpv')} variant="secondary" size="sm" onclick={onOpen}>Open in mpv</LinkButton>
			<LinkButton href={href('vlc')} variant="secondary" size="sm" onclick={onOpen}>Open in VLC</LinkButton>
		</div>
		<details class="text-xs text-muted">
			<summary class="cursor-pointer">Set up external players on this computer</summary>
			<p class="mt-1">Install the Lain CLI and mpv or VLC, then run:</p>
			<code class="mt-1 block select-all break-all">lain login --server {origin}</code>
			<code class="block select-all">lain install-player-handler</code>
			<p class="mt-1">Your browser may ask before opening the player. Playback uses the local CLI login and saves watch progress.</p>
		</details>
	</div>
{/if}
