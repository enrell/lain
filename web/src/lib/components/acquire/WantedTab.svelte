<script lang="ts">
	/*
	 * Wanted (docs/slices/acquisition.md, Phase 2): monitored titles and
	 * what they still need. Keys: m monitors a title, r syncs RSS now; a
	 * focused row takes s (search now), e (edit), Space (pause/resume
	 * monitoring), t (subtitles), Delete (stop monitoring), j/k (move).
	 */
	import { onMount } from 'svelte';
	import { api, type AutomationState, type Library, type Monitored, type Profile, type WantedRow } from '$lib/api';
	import Button from '$lib/components/primitives/Button.svelte';
	import Modal from '$lib/components/primitives/Modal.svelte';
	import Spinner from '$lib/components/primitives/Spinner.svelte';
	import { compressUnits, unitLabel } from '$lib/acquire/format';
	import { t } from '$lib/i18n';
	import { toasts } from '$lib/stores/toasts.svelte';
	import { errorMessage } from '$lib/utilities/errors';
	import { formatRelative } from '$lib/utilities/format';
	import { isTypingTarget } from '$lib/utilities/guards';
	import MonitorDialog from './MonitorDialog.svelte';
	import SubtitlePanel from './SubtitlePanel.svelte';

	let { libraries, prefill = $bindable('') }: { libraries: Library[]; prefill?: string } = $props();

	let rows = $state<WantedRow[] | null>(null);
	let automation = $state<AutomationState | null>(null);
	let profiles = $state<Profile[]>([]);
	let busy = $state('');
	let dialogOpen = $state(false);
	let editing = $state<Monitored | null>(null);
	let removing = $state<WantedRow | null>(null);
	let removeOpen = $state(false);
	let subsFor = $state<WantedRow | null>(null);
	let subsOpen = $state(false);

	const libraryName = (id: string) => libraries.find((l) => l.id === id)?.name ?? id;
	const profileName = (id: string) => profiles.find((p) => p.id === id)?.name ?? id;

	async function load(): Promise<void> {
		try {
			const [w, p] = await Promise.all([api.acquire.wanted(), api.acquire.profiles()]);
			rows = w.titles;
			automation = w.automation;
			profiles = p.profiles;
		} catch (err) {
			toasts.error(errorMessage(err, t('acquire.wanted.loadFailed')));
			rows = [];
		}
	}
	onMount(() => {
		void load();
		if (prefill) openDialog(null);
	});

	function openDialog(m: Monitored | null): void {
		editing = m;
		dialogOpen = true;
	}

	function report(r: { grabbed: string[]; considered: number; rejected: number; paused?: string }): void {
		if (r.paused) toasts.error(r.paused);
		else toasts.success(t('acquire.wanted.report', { grabbed: r.grabbed.length, considered: r.considered, rejected: r.rejected }));
	}

	async function search(row: WantedRow): Promise<void> {
		busy = row.id;
		try {
			report(await api.acquire.searchMonitored(row.id));
			await load();
		} catch (err) {
			toasts.error(errorMessage(err, t('acquire.wanted.searchFailed')));
		} finally {
			busy = '';
		}
	}

	async function rss(): Promise<void> {
		busy = 'rss';
		try {
			report(await api.acquire.rss());
			await load();
		} catch (err) {
			toasts.error(errorMessage(err, t('acquire.wanted.searchFailed')));
		} finally {
			busy = '';
		}
	}

	async function toggle(row: WantedRow): Promise<void> {
		try {
			const { wanted: _w, missing_count: _m, upgrade_count: _u, ...mon } = row;
			await api.acquire.updateMonitored(row.id, { ...mon, enabled: !row.enabled });
			await load();
		} catch (err) {
			toasts.error(errorMessage(err, t('acquire.monitor.failed')));
		}
	}

	function askRemove(row: WantedRow): void {
		removing = row;
		removeOpen = true;
		requestAnimationFrame(() => requestAnimationFrame(() => document.getElementById('confirm-unmonitor')?.focus()));
	}

	async function remove(): Promise<void> {
		if (!removing) return;
		try {
			await api.acquire.unmonitor(removing.id);
			removeOpen = false;
			await load();
		} catch (err) {
			toasts.error(errorMessage(err, t('acquire.monitor.failed')));
		}
	}

	function rowKey(e: KeyboardEvent, row: WantedRow): void {
		if (e.ctrlKey || e.metaKey || e.altKey) return;
		const el = e.currentTarget as HTMLElement;
		switch (e.key) {
			case 's':
				void search(row);
				break;
			case 'e':
			case 'Enter':
				openDialog(row);
				break;
			case 't':
				subsFor = row;
				subsOpen = true;
				break;
			case ' ':
				void toggle(row);
				break;
			case 'Delete':
				askRemove(row);
				break;
			case 'j':
			case 'ArrowDown':
				(el.nextElementSibling as HTMLElement | null)?.focus();
				break;
			case 'k':
			case 'ArrowUp':
				(el.previousElementSibling as HTMLElement | null)?.focus();
				break;
			default:
				return;
		}
		e.preventDefault();
	}

	function pageKey(e: KeyboardEvent): void {
		if (e.ctrlKey || e.metaKey || e.altKey || isTypingTarget(e.target) || document.querySelector('[role="dialog"]')) return;
		if (e.key === 'm') {
			e.preventDefault();
			openDialog(null);
		} else if (e.key === 'r') {
			e.preventDefault();
			void rss();
		}
	}

	function openEdge(row: WantedRow): string {
		const w = row.wanted;
		if (w.open_from) return t('acquire.wanted.openFrom', { unit: unitLabel(w.open_from, row.numbering) });
		const seasons = Object.entries(w.open_seasons ?? {});
		if (seasons.length) return t('acquire.wanted.openFrom', { unit: seasons.map(([s, n]) => unitLabel({ season: Number(s), number: n }, 'seasonal')).join(', ') });
		return '';
	}
</script>

<svelte:window onkeydown={pageKey} />

<div class="flex flex-wrap items-center justify-between gap-3">
	<p class="font-mono text-[11px] text-muted">
		{#if automation?.paused}
			<span class="text-warning">{t('acquire.wanted.paused', { reason: automation.paused })}</span>
		{:else if automation?.enabled}
			{t('acquire.wanted.automationOn', { when: automation.last_rss_at ? formatRelative(automation.last_rss_at) : t('acquire.wanted.never') })}
		{:else}
			{t('acquire.wanted.automationOff')} <a href="/settings/acquisition#automation" class="text-accent">{t('acquire.settingsLink')}</a>
		{/if}
	</p>
	<div class="flex gap-2">
		<Button size="sm" variant="secondary" loading={busy === 'rss'} onclick={() => void rss()}>{t('acquire.wanted.rss')} <kbd class="ms-1 font-mono text-[10px] text-muted">r</kbd></Button>
		<Button size="sm" onclick={() => openDialog(null)}>{t('acquire.wanted.monitor')} <kbd class="ms-1 font-mono text-[10px] opacity-70">m</kbd></Button>
	</div>
</div>

{#if rows === null}
	<div class="flex justify-center py-10"><Spinner class="size-5 text-muted" /></div>
{:else if rows.length === 0}
	<p class="py-6 text-sm text-muted">{t('acquire.wanted.empty')}</p>
{:else}
	<p class="text-xs text-muted">{t('acquire.wanted.keys')}</p>
	<div class="divide-y divide-hairline" role="grid" aria-label={t('acquire.tab.wanted')}>
		{#each rows as row (row.id)}
			<div role="row" tabindex="0" onkeydown={(e) => rowKey(e, row)} class="grid gap-x-6 gap-y-1 py-3 outline-none focus-visible:bg-surface-active/40 sm:grid-cols-[minmax(0,1fr)_auto] sm:items-center {row.enabled ? '' : 'opacity-60'}">
				<div class="min-w-0" role="gridcell">
					<p class="flex flex-wrap items-baseline gap-x-3 text-sm font-medium text-foreground">
						{row.title}
						<span class="font-mono text-[9px] uppercase tracking-[0.2em] text-muted">{t(`acquire.numbering.${row.numbering}`)}</span>
						{#if !row.enabled}<span class="font-mono text-[9px] uppercase tracking-[0.2em] text-warning">{t('acquire.wanted.disabled')}</span>{/if}
					</p>
					<p class="mt-0.5 flex flex-wrap gap-x-3 font-mono text-[10px] text-muted">
						<span>{libraryName(row.library_id)}</span>
						<span>{profileName(row.profile_id)}</span>
						<span>{t('acquire.wanted.present', { count: row.wanted.present })}</span>
						{#if row.metadata_episodes}<span>{t('acquire.wanted.total', { count: row.metadata_episodes })}</span>{/if}
						{#if row.last_search_at}<span>{t('acquire.wanted.searched', { when: formatRelative(row.last_search_at) })}</span>{/if}
					</p>
					{#if row.missing_count > 0}
						<p class="mt-1 text-xs text-foreground/90">{t('acquire.wanted.missing', { count: row.missing_count, units: compressUnits(row.wanted.missing, row.numbering) })}</p>
					{/if}
					{#if row.upgrade_count > 0}
						<p class="mt-0.5 text-xs text-muted">{t('acquire.wanted.upgrades', { count: row.upgrade_count, units: compressUnits(row.wanted.upgrades.map((u) => u.unit), row.numbering) })}</p>
					{/if}
					{#if openEdge(row)}<p class="mt-0.5 text-xs text-muted">{openEdge(row)}</p>{/if}
				</div>
				<div class="flex flex-wrap items-center gap-1.5" role="gridcell">
					<Button size="sm" variant="ghost" tabindex={-1} loading={busy === row.id} onclick={() => void search(row)}>{t('acquire.wanted.search')} <kbd class="ms-1 font-mono text-[10px] text-muted">s</kbd></Button>
					<Button size="sm" variant="ghost" tabindex={-1} onclick={() => { subsFor = row; subsOpen = true; }}>{t('acquire.wanted.subtitles')} <kbd class="ms-1 font-mono text-[10px] text-muted">t</kbd></Button>
					<Button size="sm" variant="ghost" tabindex={-1} onclick={() => openDialog(row)}>{t('acquire.indexers.edit')} <kbd class="ms-1 font-mono text-[10px] text-muted">e</kbd></Button>
					<Button size="sm" variant="ghost" tabindex={-1} onclick={() => askRemove(row)}>{t('acquire.wanted.unmonitor')} <kbd class="ms-1 font-mono text-[10px] text-muted">Del</kbd></Button>
				</div>
			</div>
		{/each}
	</div>
{/if}

{#if subsFor}<SubtitlePanel bind:open={subsOpen} monitoredId={subsFor.id} title={subsFor.title} />{/if}

<MonitorDialog bind:open={dialogOpen} {libraries} {profiles} {editing} {prefill} onsaved={() => { prefill = ''; void load(); }} />

<Modal bind:open={removeOpen} title={t('acquire.wanted.unmonitorTitle', { title: removing?.title ?? '' })} description={t('acquire.wanted.unmonitorBody')}>
	{#snippet footer()}
		<Button variant="ghost" onclick={() => (removeOpen = false)}>{t('common.cancel')} <kbd class="ms-1 font-mono text-[10px] text-muted">Esc</kbd></Button>
		<Button id="confirm-unmonitor" variant="danger" onclick={() => void remove()}>{t('acquire.wanted.unmonitor')} <kbd class="ms-1 font-mono text-[10px] opacity-70">Enter</kbd></Button>
	{/snippet}
</Modal>
