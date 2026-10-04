<script lang="ts">
	import { onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import Button from '$lib/components/primitives/Button.svelte';
	import SettingRow from '$lib/components/settings/SettingRow.svelte';
	import SettingsGroup from '$lib/components/settings/SettingsGroup.svelte';
	import { t } from '$lib/i18n';
	import { loadCap, saveCap } from '$lib/offline/budget';
	import { offlineStore, offlineSupported, type SavedItem } from '$lib/offline/store';
	import { readingLabel } from '$lib/reader/kinds';
	import { toasts } from '$lib/stores/toasts.svelte';
	import { errorMessage } from '$lib/utilities/errors';
	import { formatBytes, formatRelative } from '$lib/utilities/format';
	import { fromGiB, toGiB } from '../downloads/downloads';

	/*
	 * Offline copies kept in this browser (docs/slices/web-offline.md).
	 * A personal setting: the limit applies instantly (D-087). Rows take
	 * Enter (read) and Delete (remove); j/k move between them.
	 */
	const supported = offlineSupported();
	let items = $state<SavedItem[]>([]);
	let used = $state(0);
	let budget = $state(Number.POSITIVE_INFINITY);
	let cap = $state(toGiB(loadCap()));
	let capSaved = $state(false);

	async function refresh(): Promise<void> {
		if (!supported) return;
		const store = offlineStore();
		items = await store.list();
		({ used, budget } = await store.usage());
	}

	onMount(() => void refresh());

	function applyCap(raw: string): void {
		cap = raw;
		saveCap(fromGiB(raw));
		capSaved = true;
		setTimeout(() => (capSaved = false), 1200);
		void refresh();
	}

	async function remove(s: SavedItem): Promise<void> {
		try {
			await offlineStore().remove(s.item_id);
			toasts.success(t('offline.removedToast', { name: s.title }));
			await refresh();
		} catch (err) {
			toasts.error(errorMessage(err, t('offline.failed')));
		}
	}

	async function clearAll(): Promise<void> {
		await offlineStore().clear();
		toasts.success(t('offline.storage.cleared'));
		await refresh();
	}

	function rowKey(e: KeyboardEvent, s: SavedItem): void {
		if (e.ctrlKey || e.metaKey || e.altKey) return;
		const row = e.currentTarget as HTMLElement;
		if (e.key === 'Enter') {
			e.preventDefault();
			void goto(`/read/${encodeURIComponent(s.item_id)}`);
		} else if (e.key === 'Delete') {
			e.preventDefault();
			void remove(s);
		} else if (e.key === 'ArrowDown' || e.key === 'j') {
			e.preventDefault();
			(row.nextElementSibling as HTMLElement | null)?.focus();
		} else if (e.key === 'ArrowUp' || e.key === 'k') {
			e.preventDefault();
			(row.previousElementSibling as HTMLElement | null)?.focus();
		}
	}
</script>

<svelte:head><title>{t('offline.title')}</title></svelte:head>

<div class="max-w-4xl pb-24">
	{#if !supported}
		<p class="text-sm text-muted">{t('offline.unsupported')}</p>
	{:else}
		<SettingsGroup id="storage" title={t('offline.storage.group')}>
			<SettingRow label={t('offline.storage.used')} hint={t('offline.storage.usedHint')}>
				<span class="font-mono text-xs text-foreground">
					{Number.isFinite(budget)
						? t('offline.storage.usedValue', { used: formatBytes(used) || '0 B', budget: formatBytes(budget) || '0 B' })
						: t('offline.storage.usedUnlimited', { used: formatBytes(used) || '0 B' })}
				</span>
			</SettingRow>
			<SettingRow label={t('offline.storage.cap')} hint={t('offline.storage.capHint')} saved={capSaved}>
				<input
					type="number"
					min="0"
					step="1"
					class="h-9 w-32 rounded-md border border-transparent bg-field px-3 text-end font-mono text-xs text-foreground focus:border-accent/60 focus:outline-none"
					aria-label={t('offline.storage.cap')}
					value={cap}
					onchange={(e) => applyCap(e.currentTarget.value)}
				/>
			</SettingRow>
			<SettingRow label={t('offline.storage.clear')} hint={t('offline.storage.clearHint')}>
				<Button variant="secondary" size="sm" disabled={items.length === 0} onclick={() => void clearAll()}>
					{t('offline.storage.clear')}
				</Button>
			</SettingRow>
		</SettingsGroup>

		<SettingsGroup id="items" title={t('offline.items.group')}>
			{#if items.length === 0}
				<p class="py-6 text-sm text-muted">{t('offline.items.empty', { key: 'o' })}</p>
			{:else}
				<p class="pt-4 text-xs text-muted">{t('offline.items.keys', { open: 'Enter', remove: 'Delete' })}</p>
				<div class="mt-2 divide-y divide-hairline" role="grid" aria-label={t('offline.items.group')}>
					{#each items as s (s.item_id)}
						<div
							role="row"
							tabindex="0"
							class="grid gap-x-6 gap-y-2 py-3 outline-none focus-visible:bg-surface-active/40 sm:grid-cols-[minmax(0,1fr)_auto] sm:items-center"
							onkeydown={(e) => rowKey(e, s)}
						>
							<div class="min-w-0" role="gridcell">
								<p class="truncate text-sm font-medium text-foreground">{s.title} · {readingLabel(s)}</p>
								<p class="mt-0.5 font-mono text-[10px] uppercase tracking-[0.14em] text-muted">
									{t('offline.items.pages', { count: s.pages })} · {formatBytes(s.bytes)} · {formatRelative(s.saved_at)}
								</p>
							</div>
							<div class="flex items-center gap-1.5" role="gridcell">
								<Button size="sm" variant="ghost" tabindex={-1} onclick={() => void goto(`/read/${encodeURIComponent(s.item_id)}`)}>
									{t('offline.items.open')} <kbd class="ms-1 font-mono text-[10px] text-muted">Enter</kbd>
								</Button>
								<Button size="sm" variant="ghost" tabindex={-1} onclick={() => void remove(s)}>
									{t('offline.remove')} <kbd class="ms-1 font-mono text-[10px] text-muted">Delete</kbd>
								</Button>
							</div>
						</div>
					{/each}
				</div>
			{/if}
		</SettingsGroup>
	{/if}
</div>
