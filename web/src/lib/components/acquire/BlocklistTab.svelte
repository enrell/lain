<script lang="ts">
	/*
	 * Blocklist and held files (docs/slices/acquisition.md, A-21/A-22).
	 * Blocked releases are never grabbed by automation; a focused row
	 * takes Delete (unblock) and j/k. Files an upgrade replaced are held,
	 * never deleted automatically; purging them is explicit (P, confirm).
	 */
	import { onMount } from 'svelte';
	import { api, type BlockEntry, type HeldFile } from '$lib/api';
	import Button from '$lib/components/primitives/Button.svelte';
	import Modal from '$lib/components/primitives/Modal.svelte';
	import Spinner from '$lib/components/primitives/Spinner.svelte';
	import { t } from '$lib/i18n';
	import { toasts } from '$lib/stores/toasts.svelte';
	import { errorMessage } from '$lib/utilities/errors';
	import { formatBytes, formatRelative } from '$lib/utilities/format';
	import { isTypingTarget } from '$lib/utilities/guards';

	let entries = $state<BlockEntry[] | null>(null);
	let held = $state<HeldFile[]>([]);
	let purgeOpen = $state(false);
	let purging = $state(false);

	async function load(): Promise<void> {
		try {
			const [b, h] = await Promise.all([api.acquire.blocklist(), api.acquire.replaced()]);
			entries = b.blocklist;
			held = h.files;
		} catch (err) {
			toasts.error(errorMessage(err, t('acquire.blocklist.loadFailed')));
			entries = [];
		}
	}
	onMount(() => void load());

	async function unblock(e: BlockEntry): Promise<void> {
		try {
			await api.acquire.unblock(e.id);
			await load();
		} catch (err) {
			toasts.error(errorMessage(err, t('acquire.queue.actionFailed')));
		}
	}

	function askPurge(): void {
		if (held.length === 0) return;
		purgeOpen = true;
		requestAnimationFrame(() => requestAnimationFrame(() => document.getElementById('confirm-purge')?.focus()));
	}

	async function purge(): Promise<void> {
		purging = true;
		try {
			const r = await api.acquire.purgeReplaced();
			purgeOpen = false;
			toasts.success(t('acquire.blocklist.purged', { count: r.deleted }));
			await load();
		} catch (err) {
			toasts.error(errorMessage(err, t('acquire.queue.actionFailed')));
		} finally {
			purging = false;
		}
	}

	function rowKey(e: KeyboardEvent, entry: BlockEntry): void {
		if (e.ctrlKey || e.metaKey || e.altKey) return;
		const el = e.currentTarget as HTMLElement;
		if (e.key === 'Delete') void unblock(entry);
		else if (e.key === 'j' || e.key === 'ArrowDown') (el.nextElementSibling as HTMLElement | null)?.focus();
		else if (e.key === 'k' || e.key === 'ArrowUp') (el.previousElementSibling as HTMLElement | null)?.focus();
		else return;
		e.preventDefault();
	}

	function pageKey(e: KeyboardEvent): void {
		if (e.key !== 'P' || e.ctrlKey || e.metaKey || e.altKey || isTypingTarget(e.target) || document.querySelector('[role="dialog"]')) return;
		e.preventDefault();
		askPurge();
	}
	const heldBytes = $derived(held.reduce((n, h) => n + h.size, 0));
</script>

<svelte:window onkeydown={pageKey} />

<section class="space-y-2">
	<h2 class="border-b border-hairline pb-2 font-mono text-[10px] font-semibold uppercase tracking-[0.26em] text-muted">{t('acquire.blocklist.group')}</h2>
	{#if entries === null}
		<div class="flex justify-center py-6"><Spinner class="size-5 text-muted" /></div>
	{:else if entries.length === 0}
		<p class="py-3 text-sm text-muted">{t('acquire.blocklist.empty')}</p>
	{:else}
		<p class="text-xs text-muted">{t('acquire.blocklist.keys')}</p>
		<div class="divide-y divide-hairline" role="grid" aria-label={t('acquire.blocklist.group')}>
			{#each entries as e (e.id)}
				<div role="row" tabindex="0" onkeydown={(ev) => rowKey(ev, e)} class="flex flex-wrap items-center justify-between gap-3 py-2.5 outline-none focus-visible:bg-surface-active/40">
					<div class="min-w-0" role="gridcell">
						<p class="truncate text-sm text-foreground" title={e.title}>{e.title}</p>
						<p class="font-mono text-[10px] text-muted">{e.reason} · {formatRelative(e.at)}</p>
					</div>
					<Button size="sm" variant="ghost" tabindex={-1} onclick={() => void unblock(e)}>{t('acquire.blocklist.unblock')} <kbd class="ms-1 font-mono text-[10px] text-muted">Del</kbd></Button>
				</div>
			{/each}
		</div>
	{/if}
</section>

<section class="space-y-2 pt-6">
	<div class="flex items-center justify-between border-b border-hairline pb-2">
		<h2 class="font-mono text-[10px] font-semibold uppercase tracking-[0.26em] text-muted">{t('acquire.blocklist.heldGroup')}</h2>
		<Button size="sm" variant="ghost" disabled={held.length === 0} onclick={askPurge}>{t('acquire.blocklist.purge')} <kbd class="ms-1 font-mono text-[10px] text-muted">Shift+P</kbd></Button>
	</div>
	<p class="text-xs text-muted">{t('acquire.blocklist.heldHint', { count: held.length, size: formatBytes(heldBytes) || '0 B' })}</p>
	<ul class="divide-y divide-hairline">
		{#each held as h (h.path)}
			<li class="flex items-center justify-between gap-3 py-2 font-mono text-[11px] text-muted">
				<span class="truncate" title={h.path}>{h.path}</span>
				<span class="shrink-0">{formatBytes(h.size)}</span>
			</li>
		{/each}
	</ul>
</section>

<Modal bind:open={purgeOpen} title={t('acquire.blocklist.purgeTitle', { count: held.length })} description={t('acquire.blocklist.purgeBody')}>
	{#snippet footer()}
		<Button variant="ghost" onclick={() => (purgeOpen = false)}>{t('common.cancel')} <kbd class="ms-1 font-mono text-[10px] text-muted">Esc</kbd></Button>
		<Button id="confirm-purge" variant="danger" loading={purging} onclick={() => void purge()}>{t('acquire.blocklist.purge')} <kbd class="ms-1 font-mono text-[10px] opacity-70">Enter</kbd></Button>
	{/snippet}
</Modal>
