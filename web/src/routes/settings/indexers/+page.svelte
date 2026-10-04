<script lang="ts">
	import { onMount } from 'svelte';
	import { api, type Indexer, type IndexerInput } from '$lib/api';
	import Button from '$lib/components/primitives/Button.svelte';
	import Modal from '$lib/components/primitives/Modal.svelte';
	import Spinner from '$lib/components/primitives/Spinner.svelte';
	import Switch from '$lib/components/primitives/Switch.svelte';
	import SettingRow from '$lib/components/settings/SettingRow.svelte';
	import SettingsGroup from '$lib/components/settings/SettingsGroup.svelte';
	import { parseCategories } from '$lib/acquire/format';
	import { t } from '$lib/i18n';
	import { toasts } from '$lib/stores/toasts.svelte';
	import { errorMessage } from '$lib/utilities/errors';
	import { formatRelative } from '$lib/utilities/format';
	import { isTypingTarget } from '$lib/utilities/guards';

	/*
	 * Indexers (docs/slices/acquisition.md, A-7/A-8): Torznab and Newznab,
	 * so direct indexers and Jackett/Prowlarr both work. Changes apply at
	 * once. Keys: n adds; a focused row takes t (test), e (edit),
	 * Delete (remove), j/k (move). API keys are write-only.
	 */
	let indexers = $state<Indexer[] | null>(null);
	let busy = $state('');
	let editing = $state<Indexer | null>(null);
	let open = $state(false);
	let saving = $state(false);
	let confirmDelete = $state<Indexer | null>(null);
	let deleteOpen = $state(false);
	let nameInput = $state<HTMLInputElement>();

	let form = $state({ name: '', protocol: 'torznab' as 'torznab' | 'newznab', url: '', api_key: '', categories: '', enabled: true, min_interval_ms: 2000 });

	async function load(): Promise<void> {
		try {
			indexers = (await api.acquire.indexers()).indexers;
		} catch (err) {
			toasts.error(errorMessage(err, t('acquire.indexers.loadFailed')));
			indexers = [];
		}
	}
	onMount(() => void load());

	function startEdit(ix: Indexer | null): void {
		editing = ix;
		form = ix
			? { name: ix.name, protocol: ix.protocol, url: ix.url, api_key: '', categories: ix.categories.join(', '), enabled: ix.enabled, min_interval_ms: ix.min_interval_ms }
			: { name: '', protocol: 'torznab', url: '', api_key: '', categories: '', enabled: true, min_interval_ms: 2000 };
		open = true;
		requestAnimationFrame(() => requestAnimationFrame(() => nameInput?.focus()));
	}

	async function submit(e: SubmitEvent): Promise<void> {
		e.preventDefault();
		saving = true;
		const body: IndexerInput = {
			name: form.name, protocol: form.protocol, url: form.url, categories: parseCategories(form.categories),
			enabled: form.enabled, min_interval_ms: Number(form.min_interval_ms) || 0
		};
		if (form.api_key.trim()) body.api_key = form.api_key.trim();
		try {
			const saved = editing ? await api.acquire.updateIndexer(editing.id, body) : await api.acquire.createIndexer(body);
			open = false;
			toasts.success(t('acquire.indexers.saved', { name: saved.name }));
			await load();
			if (!editing) void test(saved);
		} catch (err) {
			toasts.error(errorMessage(err, t('acquire.indexers.saveFailed')));
		} finally {
			saving = false;
		}
	}

	async function test(ix: Indexer): Promise<void> {
		busy = ix.id;
		try {
			const res = await api.acquire.testIndexer(ix.id);
			if (res.ok) toasts.success(t('acquire.indexers.testOk', { name: ix.name }));
			else toasts.error(t('acquire.indexers.testFailed', { name: ix.name, error: res.error ?? '' }));
			await load();
		} catch (err) {
			toasts.error(errorMessage(err, t('acquire.indexers.testFailed', { name: ix.name, error: '' })));
		} finally {
			busy = '';
		}
	}

	async function toggle(ix: Indexer, enabled: boolean): Promise<void> {
		try {
			await api.acquire.updateIndexer(ix.id, { enabled });
			await load();
		} catch (err) {
			toasts.error(errorMessage(err, t('acquire.indexers.saveFailed')));
		}
	}

	async function remove(ix: Indexer): Promise<void> {
		try {
			await api.acquire.deleteIndexer(ix.id);
			deleteOpen = false;
			toasts.success(t('acquire.indexers.removed', { name: ix.name }));
			await load();
		} catch (err) {
			toasts.error(errorMessage(err, t('acquire.indexers.saveFailed')));
		}
	}

	function askDelete(ix: Indexer): void {
		confirmDelete = ix;
		deleteOpen = true;
		requestAnimationFrame(() => requestAnimationFrame(() => document.getElementById('confirm-remove-indexer')?.focus()));
	}

	function rowKey(e: KeyboardEvent, ix: Indexer): void {
		if (e.ctrlKey || e.metaKey || e.altKey) return;
		const row = e.currentTarget as HTMLElement;
		switch (e.key) {
			case 't':
				void test(ix);
				break;
			case 'e':
			case 'Enter':
				startEdit(ix);
				break;
			case 'Delete':
				askDelete(ix);
				break;
			case 'j':
			case 'ArrowDown':
				(row.nextElementSibling as HTMLElement | null)?.focus();
				break;
			case 'k':
			case 'ArrowUp':
				(row.previousElementSibling as HTMLElement | null)?.focus();
				break;
			default:
				return;
		}
		e.preventDefault();
	}

	function pageKey(e: KeyboardEvent): void {
		if (e.key !== 'n' || e.ctrlKey || e.metaKey || e.altKey || isTypingTarget(e.target) || document.querySelector('[role="dialog"]')) return;
		e.preventDefault();
		startEdit(null);
	}

	const field = 'h-10 w-full rounded-md border border-transparent bg-field px-3 text-sm text-foreground placeholder:text-muted/60 focus:border-accent/60 focus:outline-none';
</script>

<svelte:head><title>{t('acquire.indexers.title')}</title></svelte:head>
<svelte:window onkeydown={pageKey} />

<div class="max-w-4xl pb-24">
	<SettingsGroup id="indexers" title={t('acquire.indexers.group')}>
		{#snippet aside()}
			<Button size="sm" onclick={() => startEdit(null)}>{t('acquire.indexers.add')} <kbd class="ms-1 font-mono text-[10px] opacity-70">n</kbd></Button>
		{/snippet}
		{#if indexers === null}
			<div class="flex justify-center py-10"><Spinner class="size-5 text-muted" /></div>
		{:else if indexers.length === 0}
			<p class="py-6 text-sm text-muted">{t('acquire.indexers.empty')}</p>
		{:else}
			<p class="pt-4 text-xs text-muted">{t('acquire.indexers.keys')}</p>
			<div class="mt-2 divide-y divide-hairline" role="grid" aria-label={t('acquire.indexers.group')}>
				{#each indexers as ix (ix.id)}
					<div role="row" tabindex="0" class="grid gap-x-6 gap-y-2 py-3 outline-none focus-visible:bg-surface-active/40 sm:grid-cols-[minmax(0,1fr)_auto] sm:items-center" onkeydown={(e) => rowKey(e, ix)}>
						<div class="min-w-0" role="gridcell">
							<p class="flex items-center gap-3 text-sm font-medium text-foreground">
								{ix.name}
								<span class="font-mono text-[9px] uppercase tracking-[0.2em] text-muted">{ix.protocol}</span>
								{#if !ix.has_api_key}<span class="font-mono text-[9px] uppercase tracking-[0.2em] text-warning">{t('acquire.indexers.noKey')}</span>{/if}
							</p>
							<p class="mt-0.5 truncate font-mono text-[11px] text-muted">{ix.url}</p>
							<p class="mt-0.5 font-mono text-[10px] text-muted">
								{#if ix.last_error}
									<span class="text-danger">{ix.last_error}</span>
								{:else if ix.last_check_at}
									{t('acquire.indexers.healthy', { when: formatRelative(ix.last_check_at), count: ix.caps?.categories.length ?? 0 })}
								{:else}
									{t('acquire.indexers.untested')}
								{/if}
							</p>
						</div>
						<div class="flex flex-wrap items-center gap-1.5" role="gridcell">
							<Switch bare label={t('acquire.indexers.enabled')} checked={ix.enabled} onCheckedChange={(v) => void toggle(ix, v)} />
							<Button size="sm" variant="ghost" tabindex={-1} loading={busy === ix.id} onclick={() => void test(ix)}>{t('acquire.indexers.test')} <kbd class="ms-1 font-mono text-[10px] text-muted">t</kbd></Button>
							<Button size="sm" variant="ghost" tabindex={-1} onclick={() => startEdit(ix)}>{t('acquire.indexers.edit')} <kbd class="ms-1 font-mono text-[10px] text-muted">e</kbd></Button>
							<Button size="sm" variant="ghost" tabindex={-1} onclick={() => askDelete(ix)}>{t('acquire.indexers.remove')} <kbd class="ms-1 font-mono text-[10px] text-muted">Del</kbd></Button>
						</div>
					</div>
				{/each}
			</div>
		{/if}
		<SettingRow label={t('acquire.indexers.aboutLabel')} hint={t('acquire.indexers.aboutHint')} />
	</SettingsGroup>
</div>

<Modal bind:open title={editing ? t('acquire.indexers.editTitle', { name: editing.name }) : t('acquire.indexers.addTitle')} description={t('acquire.indexers.formHint')}>
	<form id="indexer-form" class="space-y-3" onsubmit={submit}>
		<label class="block space-y-1.5"><span class="text-xs text-muted">{t('acquire.indexers.name')}</span>
			<input bind:this={nameInput} bind:value={form.name} maxlength="60" required class={field} placeholder="tracker-exemplo" />
		</label>
		<label class="block space-y-1.5"><span class="text-xs text-muted">{t('acquire.indexers.protocol')}</span>
			<select bind:value={form.protocol} class={field}>
				<option value="torznab">Torznab</option>
				<option value="newznab">Newznab</option>
			</select>
		</label>
		<label class="block space-y-1.5"><span class="text-xs text-muted">{t('acquire.indexers.url')}</span>
			<input bind:value={form.url} type="url" required spellcheck={false} class="{field} font-mono text-xs" placeholder="http://127.0.0.1:9117/api/v2.0/indexers/x/results/torznab" />
		</label>
		<label class="block space-y-1.5"><span class="text-xs text-muted">{editing?.has_api_key ? t('acquire.indexers.apiKeyKeep') : t('acquire.indexers.apiKey')}</span>
			<input bind:value={form.api_key} type="password" autocomplete="off" spellcheck={false} class="{field} font-mono text-xs" />
		</label>
		<label class="block space-y-1.5"><span class="text-xs text-muted">{t('acquire.indexers.categories')}</span>
			<input bind:value={form.categories} spellcheck={false} class="{field} font-mono text-xs" placeholder="5070, 5000, 7030" />
		</label>
		<label class="block space-y-1.5"><span class="text-xs text-muted">{t('acquire.indexers.interval')}</span>
			<input bind:value={form.min_interval_ms} type="number" min="0" max="600000" class="{field} font-mono text-xs" />
		</label>
		<Switch label={t('acquire.indexers.enabled')} bind:checked={form.enabled} />
	</form>
	{#snippet footer()}
		<Button variant="ghost" onclick={() => (open = false)}>{t('common.cancel')} <kbd class="ms-1 font-mono text-[10px] text-muted">Esc</kbd></Button>
		<Button type="submit" form="indexer-form" loading={saving}>{t('acquire.indexers.save')} <kbd class="ms-1 font-mono text-[10px] opacity-70">Enter</kbd></Button>
	{/snippet}
</Modal>

<Modal bind:open={deleteOpen} title={t('acquire.indexers.removeTitle', { name: confirmDelete?.name ?? '' })} description={t('acquire.indexers.removeBody')}>
	{#snippet footer()}
		<Button variant="ghost" onclick={() => (deleteOpen = false)}>{t('common.cancel')} <kbd class="ms-1 font-mono text-[10px] text-muted">Esc</kbd></Button>
		<Button id="confirm-remove-indexer" variant="danger" onclick={() => confirmDelete && void remove(confirmDelete)}>{t('acquire.indexers.remove')} <kbd class="ms-1 font-mono text-[10px] opacity-70">Enter</kbd></Button>
	{/snippet}
</Modal>
