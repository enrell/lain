<script lang="ts">
	/*
	 * Other ways to play a file, as one "Open with" menu beside Play: the
	 * player on this machine when the server runs here, mpv or VLC through
	 * the CLI's link handler, and how to set that handler up.
	 */
	import { onMount } from 'svelte';
	import ExternalLink from '@lucide/svelte/icons/external-link';
	import Menu, { type MenuEntry } from '$lib/components/primitives/Menu.svelte';
	import Modal from '$lib/components/primitives/Modal.svelte';
	import type { ButtonSize, ButtonVariant } from '$lib/components/primitives/button-styles';
	import { api, ApiError } from '$lib/api';
	import { externalPlayerUrl } from '$lib/player/external-player';
	import { toasts } from '$lib/stores/toasts.svelte';
	import { errorMessage } from '$lib/utilities/errors';
	import { t } from '$lib/i18n';

	let {
		itemId,
		onOpen,
		variant = 'ghost',
		size = 'sm',
		class: className = ''
	}: { itemId: string; onOpen?: () => void; variant?: ButtonVariant; size?: ButtonSize; class?: string } = $props();
	let origin = $state('');
	let localPlayers = $state<string[]>([]);
	let setup = $state(false);
	onMount(() => {
		origin = window.location.origin;
		void api.localplay
			.players()
			.then((cap) => (localPlayers = cap.players ?? []))
			.catch(() => {});
	});

	function open(player: 'mpv' | 'vlc'): void {
		onOpen?.();
		window.location.assign(externalPlayerUrl(origin, itemId, player));
	}

	async function playLocal(): Promise<void> {
		try {
			const result = await api.localplay.play(itemId);
			toasts.success(t('openWith.playing', { player: result.player }));
		} catch (err) {
			if (err instanceof ApiError && (err.kind === 'forbidden' || err.kind === 'unavailable')) {
				localPlayers = [];
				return;
			}
			toasts.error(errorMessage(err, t('openWith.failed')));
		}
	}

	const entries = $derived<MenuEntry[]>([
		...(localPlayers.length ? [{ label: t('openWith.thisMachine'), onSelect: () => void playLocal() }] : []),
		{ label: 'mpv', onSelect: () => open('mpv') },
		{ label: 'VLC', onSelect: () => open('vlc') },
		{ label: t('openWith.setup'), onSelect: () => (setup = true), separated: true }
	]);
</script>

{#if origin}
	<Menu label={t('openWith.label')} {entries} {variant} {size} class={className} align="start" hint={t('openWith.hint')}>
		{#snippet icon()}<ExternalLink class="size-3.5" aria-hidden="true" />{/snippet}
	</Menu>
	<Modal bind:open={setup} title={t('openWith.setupTitle')} description={t('openWith.setupBody')}>
		<code class="block select-all break-all rounded-md bg-background px-3 py-2 font-mono text-xs text-foreground">lain login --server {origin}</code>
		<code class="mt-2 block select-all rounded-md bg-background px-3 py-2 font-mono text-xs text-foreground">lain install-player-handler</code>
		<p class="mt-3 text-sm text-muted">{t('openWith.setupNote')}</p>
	</Modal>
{/if}
