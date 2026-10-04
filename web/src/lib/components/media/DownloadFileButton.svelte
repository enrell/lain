<script lang="ts">
	/*
	 * Saves original files to this device through the browser (one
	 * attachment per item). With `hotkey`, `d` triggers it while no field
	 * or dialog has focus. Several items download one after another; the
	 * browser may ask once to allow multiple downloads.
	 */
	import Download from '@lucide/svelte/icons/download';
	import { session } from '$lib/auth/session.svelte';
	import Button from '$lib/components/primitives/Button.svelte';
	import type { ButtonSize, ButtonVariant } from '$lib/components/primitives/button-styles';
	import { downloadable, downloadUrl } from '$lib/download/files';
	import { t } from '$lib/i18n';

	let {
		items,
		hotkey = false,
		size = 'sm',
		variant = 'secondary'
	}: {
		items: { id: string; missing?: boolean }[];
		hotkey?: boolean;
		size?: ButtonSize;
		variant?: ButtonVariant;
	} = $props();

	const ready = $derived(downloadable(items));
	let busy = $state(false);

	async function start(): Promise<void> {
		if (busy || ready.length === 0) return;
		busy = true;
		try {
			for (const [i, item] of ready.entries()) {
				const a = document.createElement('a');
				a.href = downloadUrl(item.id, session.token);
				a.download = '';
				a.rel = 'noopener';
				document.body.appendChild(a);
				a.click();
				a.remove();
				// Browsers drop rapid-fire downloads; space them out.
				if (i < ready.length - 1) await new Promise((r) => setTimeout(r, 400));
			}
		} finally {
			busy = false;
		}
	}

	function onKey(e: KeyboardEvent): void {
		if (!hotkey || e.key !== 'd' || e.ctrlKey || e.metaKey || e.altKey || e.defaultPrevented) return;
		const el = e.target as HTMLElement | null;
		if (el && (['INPUT', 'TEXTAREA', 'SELECT'].includes(el.tagName) || el.isContentEditable)) return;
		if (document.querySelector('[role="dialog"]')) return;
		e.preventDefault();
		void start();
	}
</script>

<svelte:window onkeydown={onKey} />

<Button {variant} {size} loading={busy} disabled={ready.length === 0} onclick={() => void start()}>
	<Download class="size-3.5" aria-hidden="true" />
	{ready.length > 1 ? t('download.files', { count: ready.length }) : t('download.file')}
	{#if hotkey}<kbd class="ms-1 font-mono text-[10px] opacity-70">d</kbd>{/if}
</Button>
