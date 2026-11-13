<script lang="ts">
	import { onMount } from 'svelte';
	import { page } from '$app/state';
	import { api, type Indexer } from '$lib/api';
	import Button from '$lib/components/primitives/Button.svelte';
	import Modal from '$lib/components/primitives/Modal.svelte';
	import Spinner from '$lib/components/primitives/Spinner.svelte';
	import Switch from '$lib/components/primitives/Switch.svelte';
	import SettingRow from '$lib/components/settings/SettingRow.svelte';
	import SettingsGroup from '$lib/components/settings/SettingsGroup.svelte';
	import { t } from '$lib/i18n';
	import AddIndexer from './AddIndexer.svelte';
	import AgentSetup from './AgentSetup.svelte';
	import { toasts } from '$lib/stores/toasts.svelte';
	import { errorMessage } from '$lib/utilities/errors';
	import { formatRelative } from '$lib/utilities/format';
	import { isTypingTarget } from '$lib/utilities/guards';
	import { usePageKeys } from '$lib/stores/page-keys.svelte';

	/*
	 * Indexers (docs/slices/acquisition.md, A-7/A-8): Torznab and Newznab,
	 * so direct indexers and Jackett/Prowlarr both work. Changes apply at
	 * once. Two ways in (D-290): the Add dialog (Prowlarr and Jackett
	 * presets, or a feed tried before saving) and a coding agent with a
	 * key that only manages indexers. Keys: n adds, a sets up an agent; a
	 * focused row takes t (test), e (edit), Delete (remove), j/k (move).
	 * API keys are write-only. ?setup=add|agent opens a dialog (Acquire's
	 * first run links there).
	 */
	let indexers = $state<Indexer[] | null>(null);
	let busy = $state('');
	let editing = $state<Indexer | null>(null);
	let open = $state(false);
	let agentOpen = $state(false);
	let confirmDelete = $state<Indexer | null>(null);
	let deleteOpen = $state(false);

	async function load(): Promise<void> {
		try {
			indexers = (await api.acquire.indexers()).indexers;
		} catch (err) {
			toasts.error(errorMessage(err, t('acquire.indexers.loadFailed')));
			indexers = [];
		}
	}
	onMount(() => {
		void load();
		const setup = page.url.searchParams.get('setup');
		if (setup === 'agent') agentOpen = true;
		else if (setup === 'add') startEdit(null);
	});

	function startEdit(ix: Indexer | null): void {
		editing = ix;
		open = true;
	}

	async function saved(fresh?: Indexer): Promise<void> {
		await load();
		if (fresh) void test(fresh);
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

	usePageKeys(() => [
		{ keys: ['N'], label: t('acquire.firstRun.addIndexer') },
		{ keys: ['A'], label: t('acquire.agent.open') }
	]);

	function pageKey(e: KeyboardEvent): void {
		if ((e.key !== 'n' && e.key !== 'a') || e.ctrlKey || e.metaKey || e.altKey || isTypingTarget(e.target) || document.querySelector('[role="dialog"]')) return;
		e.preventDefault();
		if (e.key === 'n') startEdit(null);
		else agentOpen = true;
	}
</script>

<svelte:head><title>{t('acquire.indexers.title')}</title></svelte:head>
<svelte:window onkeydown={pageKey} />

<div>
	<SettingsGroup id="indexers" title={t('acquire.indexers.group')} description={t('settings.groups.indexers.indexers')}>
		{#snippet aside()}
			<div class="flex flex-wrap gap-1.5">
				<Button size="field" variant="ghost" onclick={() => (agentOpen = true)}>{t('acquire.agent.open')} <kbd class="ms-1 font-mono text-[10px] text-muted">a</kbd></Button>
				<Button size="field" variant="tonal" onclick={() => startEdit(null)}>{t('acquire.indexers.add')} <kbd class="ms-1 font-mono text-[10px] text-muted">n</kbd></Button>
			</div>
		{/snippet}
		{#if indexers === null}
			<div class="flex justify-center py-10"><Spinner class="size-5 text-muted" /></div>
		{:else if indexers.length === 0}
			<div class="space-y-3 py-5">
				<p class="text-sm text-muted">{t('acquire.indexers.empty')}</p>
				<ul class="space-y-1.5 text-sm">
					<li><button type="button" class="text-left text-foreground/85 hover:text-foreground" onclick={() => startEdit(null)}><kbd class="me-2 font-mono text-[10px] text-muted">n</kbd>{t('acquire.indexers.emptyPresets')}</button></li>
					<li><button type="button" class="text-left text-foreground/85 hover:text-foreground" onclick={() => (agentOpen = true)}><kbd class="me-2 font-mono text-[10px] text-muted">a</kbd>{t('acquire.indexers.emptyAgent')}</button></li>
				</ul>
			</div>
		{:else}
			<p class="settings-note">{t('acquire.indexers.keys')}</p>
			<div class="settings-items" role="grid" aria-label={t('acquire.indexers.group')}>
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
							<Button size="field" variant="ghost" tabindex={-1} loading={busy === ix.id} onclick={() => void test(ix)}>{t('acquire.indexers.test')} <kbd class="ms-1 font-mono text-[10px] text-muted">t</kbd></Button>
							<Button size="field" variant="ghost" tabindex={-1} onclick={() => startEdit(ix)}>{t('acquire.indexers.edit')} <kbd class="ms-1 font-mono text-[10px] text-muted">e</kbd></Button>
							<Button size="field" variant="ghost" tabindex={-1} onclick={() => askDelete(ix)}>{t('acquire.indexers.remove')} <kbd class="ms-1 font-mono text-[10px] text-muted">Del</kbd></Button>
						</div>
					</div>
				{/each}
			</div>
		{/if}
		<SettingRow label={t('acquire.indexers.aboutLabel')} hint={t('acquire.indexers.aboutHint')} />
	</SettingsGroup>
</div>

<AddIndexer bind:open {editing} onsaved={(fresh) => void saved(fresh)} />
<AgentSetup bind:open={agentOpen} />

<Modal bind:open={deleteOpen} title={t('acquire.indexers.removeTitle', { name: confirmDelete?.name ?? '' })} description={t('acquire.indexers.removeBody')}>
	{#snippet footer()}
		<Button variant="ghost" onclick={() => (deleteOpen = false)}>{t('common.cancel')} <kbd class="ms-1 font-mono text-[10px] text-muted">Esc</kbd></Button>
		<Button id="confirm-remove-indexer" variant="danger" onclick={() => confirmDelete && void remove(confirmDelete)}>{t('acquire.indexers.remove')} <kbd class="ms-1 font-mono text-[10px] opacity-70">Enter</kbd></Button>
	{/snippet}
</Modal>
