<script lang="ts">
	import { onMount } from 'svelte';
	import LinkButton from '$lib/components/primitives/LinkButton.svelte';
	import Button from '$lib/components/primitives/Button.svelte';
	import { api, ApiError } from '$lib/api';
	import { externalPlayerUrl } from '$lib/player/external-player';
	import { toasts } from '$lib/stores/toasts.svelte';
	import { errorMessage } from '$lib/utilities/errors';

	let { itemId, onOpen }: { itemId: string; onOpen?: () => void } = $props();
	let origin = $state('');
	let localPlayers = $state<string[]>([]);
	let starting = $state(false);
	onMount(() => {
		origin = window.location.origin;
		void api.localplay
			.players()
			.then((cap) => (localPlayers = cap.players))
			.catch(() => {});
	});

	function href(player: 'mpv' | 'vlc'): string { return externalPlayerUrl(origin, itemId, player); }

	async function playLocal(): Promise<void> {
		starting = true;
		try {
			const result = await api.localplay.play(itemId);
			toasts.success(`Playing in ${result.player} on this machine.`);
		} catch (err) {
			if (err instanceof ApiError && (err.kind === 'forbidden' || err.kind === 'unavailable')) {
				localPlayers = [];
				return;
			}
			toasts.error(errorMessage(err, 'Could not start local playback.'));
		} finally {
			starting = false;
		}
	}
</script>

{#if origin}
	<div class="space-y-2">
		<div class="flex flex-wrap items-center gap-2">
			{#if localPlayers.length}
				<Button variant="secondary" size="sm" loading={starting} onclick={() => void playLocal()}>
					Play on this machine
				</Button>
			{/if}
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
