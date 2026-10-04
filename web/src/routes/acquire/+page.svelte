<script lang="ts">
	/*
	 * Acquire (docs/slices/acquisition.md): search indexers, grab a
	 * release into a library, watch the queue. Admin only (A-9).
	 * Keys: 1 search, 2 queue, 3 wanted, 4 blocklist, / focuses the active
	 * tab's input. Result rows: Enter grabs, m monitors the title, j/k
	 * move. Queue rows: p pause/resume,
	 * i retry the import, Delete removes, j/k move.
	 */
	import { onDestroy, onMount, tick } from 'svelte';
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import { api, ApiError, type AcquireUsage, type Candidate, type Grab, type Indexer, type Library, type SearchResponse } from '$lib/api';
	import Button from '$lib/components/primitives/Button.svelte';
	import EmptyState from '$lib/components/primitives/EmptyState.svelte';
	import Modal from '$lib/components/primitives/Modal.svelte';
	import Spinner from '$lib/components/primitives/Spinner.svelte';
	import { grabProgress, isActiveGrab, ratio, releaseNumbers, releaseTags } from '$lib/acquire/format';
	import { t } from '$lib/i18n';
	import { toasts } from '$lib/stores/toasts.svelte';
	import { errorMessage } from '$lib/utilities/errors';
	import { formatBytes, formatRelative } from '$lib/utilities/format';
	import { isTypingTarget } from '$lib/utilities/guards';
	import BlocklistTab from '$lib/components/acquire/BlocklistTab.svelte';
	import WantedTab from '$lib/components/acquire/WantedTab.svelte';

	type Tab = 'search' | 'queue' | 'wanted' | 'blocklist';
	const TABS: Tab[] = ['search', 'queue', 'wanted', 'blocklist'];
	const tab = $derived<Tab>(TABS.find((x) => x === page.url.searchParams.get('tab')) ?? 'search');
	// A title handed from a search result to the Monitor dialog.
	let monitorPrefill = $state('');

	let libraries = $state<Library[]>([]);
	let indexers = $state<Indexer[]>([]);
	let libraryId = $state('');
	const library = $derived(libraries.find((l) => l.id === libraryId));

	// Search
	let query = $state('');
	let season = $state('');
	let episode = $state('');
	let searching = $state(false);
	let results = $state<SearchResponse | null>(null);
	let queryInput = $state<HTMLInputElement>();

	// Grab confirmation
	let pending = $state<Candidate | null>(null);
	let grabOpen = $state(false);
	let grabbing = $state(false);

	// Queue
	let grabs = $state<Grab[] | null>(null);
	let usage = $state<AcquireUsage | null>(null);
	let link = $state('');
	let linkInput = $state<HTMLInputElement>();
	let removeTarget = $state<Grab | null>(null);
	let removeOpen = $state(false);
	let removeData = $state(true);
	let timer: ReturnType<typeof setTimeout> | null = null;

	const indexerName = (id: string) => indexers.find((i) => i.id === id)?.name ?? id;
	const libraryName = (id: string) => libraries.find((l) => l.id === id)?.name ?? id;

	onMount(async () => {
		try {
			[libraries, indexers] = await Promise.all([api.libraries.list(), api.acquire.indexers().then((r) => r.indexers)]);
			libraryId = libraries[0]?.id ?? '';
		} catch (err) {
			toasts.error(errorMessage(err, t('acquire.loadFailed')));
		}
		await refresh();
	});
	onDestroy(() => {
		if (timer) clearTimeout(timer);
	});

	async function refresh(): Promise<void> {
		try {
			const res = await api.acquire.grabs();
			grabs = res.grabs;
			usage = res.usage;
		} catch (err) {
			if (grabs === null) toasts.error(errorMessage(err, t('acquire.queue.loadFailed')));
		}
		if (timer) clearTimeout(timer);
		const busy = grabs?.some((g) => isActiveGrab(g.state) || g.state === 'seeding') ?? false;
		timer = setTimeout(() => void refresh(), busy ? 1000 : 5000);
	}

	function failure(err: unknown, fallback: string): string {
		if (err instanceof ApiError && err.code === 'quota-exceeded') return t('acquire.error.quota', { detail: err.message });
		if (err instanceof ApiError && err.code === 'disk-full') return t('acquire.error.diskFull', { detail: err.message });
		return errorMessage(err, fallback);
	}

	async function search(e?: SubmitEvent): Promise<void> {
		e?.preventDefault();
		if (!query.trim() || searching) return;
		searching = true;
		try {
			results = await api.acquire.search({
				q: query.trim(), kind: library?.type, season: Number(season) || undefined, episode: Number(episode) || undefined
			});
			for (const f of results.failures) toasts.error(t('acquire.search.indexerFailed', { name: f.name, error: f.error }));
			await tick();
			document.querySelector<HTMLElement>('[data-result-row]')?.focus();
		} catch (err) {
			toasts.error(errorMessage(err, t('acquire.search.failed')));
		} finally {
			searching = false;
		}
	}

	function askGrab(c: Candidate): void {
		pending = c;
		grabOpen = true;
		requestAnimationFrame(() => requestAnimationFrame(() => document.getElementById('confirm-grab')?.focus()));
	}

	async function confirmGrab(): Promise<void> {
		if (!pending || !libraryId || grabbing) return;
		grabbing = true;
		try {
			const g = await api.acquire.grab({ result: pending, library_id: libraryId });
			grabOpen = false;
			toasts.success(t('acquire.search.grabbed', { title: g.title }));
			await refresh();
		} catch (err) {
			toasts.error(failure(err, t('acquire.search.grabFailed')));
		} finally {
			grabbing = false;
		}
	}

	async function grabLink(e: SubmitEvent): Promise<void> {
		e.preventDefault();
		const v = link.trim();
		if (!v || !libraryId) return;
		try {
			const g = await api.acquire.grab(v.startsWith('magnet:') ? { magnet: v, library_id: libraryId } : { url: v, library_id: libraryId });
			link = '';
			toasts.success(t('acquire.search.grabbed', { title: g.title }));
			await refresh();
		} catch (err) {
			toasts.error(failure(err, t('acquire.search.grabFailed')));
		}
	}

	async function act(g: Grab, action: 'pause' | 'resume' | 'import'): Promise<void> {
		try {
			await api.acquire.action(g.id, action);
			await refresh();
		} catch (err) {
			toasts.error(errorMessage(err, t('acquire.queue.actionFailed')));
		}
	}

	function askRemove(g: Grab): void {
		removeTarget = g;
		removeData = true;
		removeOpen = true;
		requestAnimationFrame(() => requestAnimationFrame(() => document.getElementById('confirm-remove-grab')?.focus()));
	}

	async function remove(): Promise<void> {
		if (!removeTarget) return;
		try {
			await api.acquire.remove(removeTarget.id, removeData);
			removeOpen = false;
			await refresh();
		} catch (err) {
			toasts.error(errorMessage(err, t('acquire.queue.actionFailed')));
		}
	}

	function move(e: KeyboardEvent): boolean {
		const row = e.currentTarget as HTMLElement;
		if (e.key === 'j' || e.key === 'ArrowDown') {
			(row.nextElementSibling as HTMLElement | null)?.focus();
			return true;
		}
		if (e.key === 'k' || e.key === 'ArrowUp') {
			(row.previousElementSibling as HTMLElement | null)?.focus();
			return true;
		}
		return false;
	}

	function resultKey(e: KeyboardEvent, c: Candidate): void {
		if (e.ctrlKey || e.metaKey || e.altKey) return;
		if (move(e)) return e.preventDefault();
		if (e.key === 'Enter' || e.key === 'g') {
			e.preventDefault();
			askGrab(c);
		} else if (e.key === 'm') {
			e.preventDefault();
			monitorPrefill = c.release.title;
			setTab('wanted');
		}
	}

	function grabKey(e: KeyboardEvent, g: Grab): void {
		if (e.ctrlKey || e.metaKey || e.altKey) return;
		if (move(e)) return e.preventDefault();
		if (e.key === 'p') {
			if (g.state === 'paused') void act(g, 'resume');
			else if (g.state === 'downloading' || g.state === 'metadata' || g.state === 'queued' || g.state === 'seeding') void act(g, 'pause');
		} else if (e.key === 'i' && g.state === 'failed' && g.finished_at && !g.data_removed) {
			void act(g, 'import');
		} else if (e.key === 'Delete' && g.state !== 'importing') {
			askRemove(g);
		} else return;
		e.preventDefault();
	}

	function setTab(next: Tab): void {
		const url = new URL(page.url);
		url.searchParams.set('tab', next);
		void goto(url, { replaceState: true, keepFocus: true, noScroll: true });
	}

	async function onKey(e: KeyboardEvent): Promise<void> {
		if (e.ctrlKey || e.metaKey || e.altKey || isTypingTarget(e.target) || document.querySelector('[role="dialog"]')) return;
		const n = Number(e.key);
		if (n >= 1 && n <= TABS.length) {
			e.preventDefault();
			setTab(TABS[n - 1]);
		} else if (e.key === '/') {
			e.preventDefault();
			await tick();
			(tab === 'search' ? queryInput : linkInput)?.focus();
		}
	}

	const field = 'h-10 rounded-md border border-transparent bg-field px-3 text-sm text-foreground placeholder:text-muted/60 focus:border-accent/60 focus:outline-none';
	const stateTone = (s: string) => (s === 'failed' ? 'text-danger' : s === 'done' || s === 'seeding' ? 'text-accent' : 'text-muted');
</script>

<svelte:head><title>{t('acquire.pageTitle')}</title></svelte:head>
<svelte:window onkeydown={(e) => void onKey(e)} />

<div class="space-y-6">
	<header class="flex flex-wrap items-end justify-between gap-4">
		<div>
			<h1 class="text-xl font-semibold tracking-tight text-foreground">{t('acquire.heading')}</h1>
			<p class="mt-0.5 text-sm text-muted">{t('acquire.subtitle')}</p>
		</div>
		<div class="flex gap-4 font-mono text-[10px] uppercase tracking-[0.2em]">
			<a href="/settings/indexers" class="text-muted hover:text-foreground">{t('acquire.indexersLink')}</a>
			<a href="/settings/acquisition" class="text-muted hover:text-foreground">{t('acquire.settingsLink')}</a>
		</div>
	</header>

	<div class="flex gap-6 border-b border-hairline" role="tablist" aria-label={t('acquire.heading')}>
		{#each [{ id: 'search', key: '1', label: t('acquire.tab.search') }, { id: 'queue', key: '2', label: t('acquire.tab.queue') }, { id: 'wanted', key: '3', label: t('acquire.tab.wanted') }, { id: 'blocklist', key: '4', label: t('acquire.tab.blocklist') }] as x (x.id)}
			<button type="button" role="tab" aria-selected={tab === x.id} onclick={() => setTab(x.id as Tab)}
				class="-mb-px flex items-center gap-2 border-b-2 pb-2 text-sm transition-colors {tab === x.id ? 'border-accent text-foreground' : 'border-transparent text-muted hover:text-foreground'}">
				{x.label}
				{#if x.id === 'queue' && grabs}<span class="font-mono text-[10px] text-muted">{grabs.filter((g) => isActiveGrab(g.state)).length}</span>{/if}
				<kbd class="rounded border border-hairline px-1 font-mono text-[10px] text-muted">{x.key}</kbd>
			</button>
		{/each}
	</div>

	{#if libraries.length === 0}
		<EmptyState title={t('acquire.noLibrariesTitle')} description={t('acquire.noLibrariesBody')}>
			<a href="/settings/libraries" class="rounded-md border border-line bg-surface px-3 py-1.5 text-xs font-medium text-foreground">{t('acquire.noLibrariesAction')}</a>
		</EmptyState>
	{:else if tab === 'search'}
		<form class="grid gap-3 sm:grid-cols-[minmax(0,1fr)_12rem_6rem_6rem_auto] sm:items-end" onsubmit={search}>
			<label class="block space-y-1.5">
				<span class="flex items-center gap-2 text-xs text-muted">{t('acquire.search.query')} <kbd class="font-mono text-[10px]">/</kbd></span>
				<input bind:this={queryInput} bind:value={query} class="{field} w-full" placeholder={t('acquire.search.placeholder')} spellcheck={false} autocomplete="off" />
			</label>
			<label class="block space-y-1.5">
				<span class="text-xs text-muted">{t('acquire.search.library')}</span>
				<select bind:value={libraryId} class="{field} w-full">
					{#each libraries as l (l.id)}<option value={l.id}>{l.name} · {l.type}</option>{/each}
				</select>
			</label>
			<label class="block space-y-1.5">
				<span class="text-xs text-muted">{library?.type === 'manga' || library?.type === 'comic' ? t('acquire.search.volume') : t('acquire.search.season')}</span>
				<input bind:value={season} type="number" min="0" class="{field} w-full font-mono" />
			</label>
			<label class="block space-y-1.5">
				<span class="text-xs text-muted">{library?.type === 'manga' || library?.type === 'comic' ? t('acquire.search.chapter') : t('acquire.search.episode')}</span>
				<input bind:value={episode} type="number" min="0" class="{field} w-full font-mono" />
			</label>
			<Button type="submit" loading={searching} disabled={!query.trim()}>{t('acquire.search.submit')} <kbd class="ms-1 font-mono text-[10px] opacity-70">Enter</kbd></Button>
		</form>

		{#if indexers.length === 0}
			<p class="text-sm text-muted">{t('acquire.search.noIndexers')} <a class="text-accent" href="/settings/indexers">{t('acquire.indexersLink')}</a></p>
		{:else if results}
			{#if results.candidates.length === 0}
				<p class="py-6 text-sm text-muted">{t('acquire.search.empty')}</p>
			{:else}
				<p class="text-xs text-muted">{t('acquire.search.keys', { count: results.candidates.length })}</p>
				<div class="divide-y divide-hairline" role="grid" aria-label={t('acquire.tab.search')}>
					<!-- Keyed by position: indexers may repeat links and omit guids, and a
					     search replaces the whole list anyway. -->
					{#each results.candidates as c, i (i)}
						<div data-result-row role="row" tabindex="0" onkeydown={(e) => resultKey(e, c)} ondblclick={() => askGrab(c)}
							class="grid gap-x-6 gap-y-1 py-3 outline-none focus-visible:bg-surface-active/40 sm:grid-cols-[minmax(0,1fr)_auto] sm:items-center {c.rejections?.length ? 'opacity-60' : ''}">
							<div class="min-w-0" role="gridcell">
								<p class="truncate text-sm text-foreground" title={c.title}>{c.title}</p>
								<p class="mt-0.5 flex flex-wrap items-center gap-x-3 font-mono text-[10px] uppercase tracking-[0.14em] text-muted">
									<span class="normal-case tracking-normal text-foreground/80">{c.release.title}</span>
									{#if releaseNumbers(c.release)}<span>{releaseNumbers(c.release)}</span>{/if}
									{#each releaseTags(c.release) as tag (tag)}<span>{tag}</span>{/each}
									{#if c.release.group}<span class="normal-case">[{c.release.group}]</span>{/if}
									<span>{indexerName(c.indexer_id)}</span>
								</p>
								{#if c.rejections?.length}<p class="mt-0.5 text-xs text-warning">{c.rejections.join(' · ')}</p>{/if}
							</div>
							<div class="flex items-center gap-4 font-mono text-[11px] text-muted" role="gridcell">
								<span>{formatBytes(c.size) || '—'}</span>
								<span title={t('acquire.search.seedersPeers')}>{c.seeders}/{c.peers}</span>
								{#if c.published_at}<span>{formatRelative(c.published_at)}</span>{/if}
								<Button size="sm" variant="ghost" tabindex={-1} onclick={() => askGrab(c)}>{t('acquire.search.grab')} <kbd class="ms-1 font-mono text-[10px] text-muted">↵</kbd></Button>
							</div>
						</div>
					{/each}
				</div>
			{/if}
		{/if}
	{:else if tab === 'wanted'}
		<WantedTab {libraries} bind:prefill={monitorPrefill} />
	{:else if tab === 'blocklist'}
		<BlocklistTab />
	{:else}
		<form class="flex flex-wrap items-end gap-3" onsubmit={grabLink}>
			<label class="block min-w-0 flex-1 space-y-1.5">
				<span class="flex items-center gap-2 text-xs text-muted">{t('acquire.queue.link')} <kbd class="font-mono text-[10px]">/</kbd></span>
				<input bind:this={linkInput} bind:value={link} class="{field} w-full font-mono text-xs" placeholder="magnet:?xt=… / https://…/file.torrent" spellcheck={false} autocomplete="off" />
			</label>
			<label class="block space-y-1.5">
				<span class="text-xs text-muted">{t('acquire.search.library')}</span>
				<select bind:value={libraryId} class={field}>
					{#each libraries as l (l.id)}<option value={l.id}>{l.name} · {l.type}</option>{/each}
				</select>
			</label>
			<Button type="submit" variant="secondary" disabled={!link.trim()}>{t('acquire.queue.addLink')} <kbd class="ms-1 font-mono text-[10px] text-muted">Enter</kbd></Button>
		</form>
		{#if usage}
			<p class="font-mono text-[11px] text-muted">
				{usage.max_bytes > 0
					? t('acquire.queue.usage', { used: formatBytes(usage.used_bytes) || '0 B', shared: formatBytes(usage.used_bytes + usage.downloads_bytes) || '0 B', max: formatBytes(usage.max_bytes) })
					: t('acquire.queue.usageUnlimited', { used: formatBytes(usage.used_bytes) || '0 B' })}
			</p>
		{/if}
		{#if grabs === null}
			<div class="flex justify-center py-10"><Spinner class="size-5 text-muted" /></div>
		{:else if grabs.length === 0}
			<p class="py-6 text-sm text-muted">{t('acquire.queue.empty')}</p>
		{:else}
			<p class="text-xs text-muted">{t('acquire.queue.keys')}</p>
			<div class="divide-y divide-hairline" role="grid" aria-label={t('acquire.tab.queue')}>
				{#each grabs as g (g.id)}
					<div role="row" tabindex="0" onkeydown={(e) => grabKey(e, g)} class="grid gap-x-6 gap-y-2 py-3 outline-none focus-visible:bg-surface-active/40 sm:grid-cols-[minmax(0,1fr)_auto] sm:items-center">
						<div class="min-w-0" role="gridcell">
							<p class="truncate text-sm font-medium text-foreground" title={g.title}>{g.title}</p>
							<p class="mt-0.5 flex flex-wrap items-center gap-x-3 font-mono text-[10px] uppercase tracking-[0.14em] text-muted">
								<span class={stateTone(g.state)}>{t(`acquire.state.${g.state}`)}</span>
								<span>{formatBytes(g.completed) || '0 B'} / {formatBytes(g.size) || '?'}</span>
								{#if g.state === 'downloading' || g.state === 'seeding'}
									<span>↓ {formatBytes(g.down_rate) || '0 B'}/s · ↑ {formatBytes(g.up_rate) || '0 B'}/s · {t('acquire.queue.peers', { count: g.peers })}</span>
								{/if}
								<span>{t('acquire.queue.ratio', { ratio: ratio(g) })}</span>
								<span class="normal-case tracking-normal">{t('acquire.queue.into', { library: libraryName(g.library_id) })}</span>
							</p>
							{#if g.error}<p class="mt-1 text-xs text-danger">{g.error}</p>{/if}
							{#if g.imported?.length}<p class="mt-1 truncate font-mono text-[10px] text-muted" title={g.imported.join('\n')}>{t('acquire.queue.imported', { count: g.imported.length, first: g.imported[0] })}</p>{/if}
							{#if isActiveGrab(g.state) || g.state === 'paused'}
								<div class="mt-2 h-1 w-full overflow-hidden rounded-full bg-line" role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(grabProgress(g) * 100)}>
									<div class="h-full bg-accent transition-[width]" style:width="{grabProgress(g) * 100}%"></div>
								</div>
							{/if}
						</div>
						<div class="flex flex-wrap items-center gap-1.5" role="gridcell">
							{#if g.state === 'paused'}
								<Button size="sm" variant="ghost" tabindex={-1} onclick={() => void act(g, 'resume')}>{t('acquire.queue.resume')} <kbd class="ms-1 font-mono text-[10px] text-muted">p</kbd></Button>
							{:else if g.state === 'downloading' || g.state === 'metadata' || g.state === 'queued' || g.state === 'seeding'}
								<Button size="sm" variant="ghost" tabindex={-1} onclick={() => void act(g, 'pause')}>{t('acquire.queue.pause')} <kbd class="ms-1 font-mono text-[10px] text-muted">p</kbd></Button>
							{/if}
							{#if g.state === 'failed' && g.finished_at && !g.data_removed}
								<Button size="sm" variant="ghost" tabindex={-1} onclick={() => void act(g, 'import')}>{t('acquire.queue.retryImport')} <kbd class="ms-1 font-mono text-[10px] text-muted">i</kbd></Button>
							{/if}
							{#if g.state !== 'importing'}
								<Button size="sm" variant="ghost" tabindex={-1} onclick={() => askRemove(g)}>{t('acquire.queue.remove')} <kbd class="ms-1 font-mono text-[10px] text-muted">Del</kbd></Button>
							{/if}
						</div>
					</div>
				{/each}
			</div>
		{/if}
	{/if}
</div>

<Modal bind:open={grabOpen} title={t('acquire.grab.title')} description={pending?.title}>
	{#if pending}
		<dl class="grid grid-cols-[8rem_minmax(0,1fr)] gap-y-2 text-sm">
			<dt class="text-muted">{t('acquire.grab.parsed')}</dt>
			<dd class="text-foreground">{pending.release.title} {releaseNumbers(pending.release)}</dd>
			<dt class="text-muted">{t('acquire.grab.size')}</dt>
			<dd class="font-mono text-xs">{formatBytes(pending.size) || '—'}</dd>
			<dt class="text-muted">{t('acquire.search.library')}</dt>
			<dd>
				<select bind:value={libraryId} class="{field} w-full">
					{#each libraries as l (l.id)}<option value={l.id}>{l.name} · {l.type}</option>{/each}
				</select>
			</dd>
		</dl>
		{#if pending.rejections?.length}<p class="mt-3 text-xs text-warning">{pending.rejections.join(' · ')}</p>{/if}
	{/if}
	{#snippet footer()}
		<Button variant="ghost" onclick={() => (grabOpen = false)}>{t('common.cancel')} <kbd class="ms-1 font-mono text-[10px] text-muted">Esc</kbd></Button>
		<Button id="confirm-grab" loading={grabbing} onclick={() => void confirmGrab()}>{t('acquire.search.grab')} <kbd class="ms-1 font-mono text-[10px] opacity-70">Enter</kbd></Button>
	{/snippet}
</Modal>

<Modal bind:open={removeOpen} title={t('acquire.queue.removeTitle', { title: removeTarget?.title ?? '' })} description={t('acquire.queue.removeBody')}>
	<label class="flex items-center gap-2 text-sm text-muted">
		<input type="checkbox" bind:checked={removeData} class="accent-[var(--color-accent)]" />
		{t('acquire.queue.removeData')}
	</label>
	{#snippet footer()}
		<Button variant="ghost" onclick={() => (removeOpen = false)}>{t('common.cancel')} <kbd class="ms-1 font-mono text-[10px] text-muted">Esc</kbd></Button>
		<Button id="confirm-remove-grab" variant="danger" onclick={() => void remove()}>{t('acquire.queue.remove')} <kbd class="ms-1 font-mono text-[10px] opacity-70">Enter</kbd></Button>
	{/snippet}
</Modal>
