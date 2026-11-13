<script lang="ts">
	/*
	 * The Add indexer dialog (D-290). Three sources: a Prowlarr or a
	 * Jackett (base URL and API key; lain asks it which indexers it
	 * serves, the owner ticks some and adds them in one step), or a feed
	 * URL typed by hand, which can be tried (caps and a few latest
	 * releases) before it is saved. Editing a saved indexer is always the
	 * feed form, tried with its stored key.
	 *
	 * No public preset: no well-known public source publishes a stable
	 * Torznab or Newznab endpoint of its own, so the dialog says to reach
	 * public ones through Prowlarr or Jackett instead of listing URLs.
	 *
	 * Keys: Enter submits (find, then add; or save), Alt+T tries the feed,
	 * the source picker is a radio group (arrows).
	 */
	import { api, type Indexer, type IndexerInput } from '$lib/api';
	import type { AggregatorSource, DiscoveredIndexer, IndexerProbe } from '$lib/api/acquire';
	import { parseCategories } from '$lib/acquire/format';
	import Button from '$lib/components/primitives/Button.svelte';
	import Modal from '$lib/components/primitives/Modal.svelte';
	import Segmented from '$lib/components/primitives/Segmented.svelte';
	import Switch from '$lib/components/primitives/Switch.svelte';
	import { focusSoon } from '$lib/components/social/focus';
	import { t } from '$lib/i18n';
	import { toasts } from '$lib/stores/toasts.svelte';
	import { errorMessage } from '$lib/utilities/errors';
	import { formatBytes } from '$lib/utilities/format';

	type Source = AggregatorSource | 'feed';

	let {
		open = $bindable(false),
		editing,
		start = 'prowlarr',
		onsaved
	}: {
		open?: boolean;
		editing: Indexer | null;
		/** The source a new indexer starts on. */
		start?: Source;
		/** After something was saved; `test` is a new indexer to test. */
		onsaved: (test?: Indexer) => void;
	} = $props();

	let source = $state<Source>('prowlarr');
	let saving = $state(false);
	let form = $state({ name: '', protocol: 'torznab' as 'torznab' | 'newznab', url: '', api_key: '', categories: '', enabled: true, min_interval_ms: 2000 });
	let agg = $state({ url: '', api_key: '' });
	// What the aggregator answered, for the URL and key it was asked with.
	let found = $state<DiscoveredIndexer[] | null>(null);
	let foundFor = $state('');
	let picked = $state<string[]>([]);
	let probing = $state(false);
	let probe = $state<IndexerProbe | null>(null);

	const DEFAULT_PORT: Record<AggregatorSource, string> = { prowlarr: 'http://127.0.0.1:9696', jackett: 'http://127.0.0.1:9117' };
	const askedWith = $derived(`${source}\n${agg.url.trim()}\n${agg.api_key.trim()}`);
	const stale = $derived(found !== null && foundFor !== askedWith);

	// Each time the dialog opens it starts over, on the indexer being edited
	// or on the start source.
	let wasOpen = false;
	$effect(() => {
		if (open && !wasOpen) reset();
		wasOpen = open;
	});

	function reset(): void {
		const ix = editing;
		source = ix ? 'feed' : start;
		form = ix
			? { name: ix.name, protocol: ix.protocol, url: ix.url, api_key: '', categories: ix.categories.join(', '), enabled: ix.enabled, min_interval_ms: ix.min_interval_ms }
			: { name: '', protocol: 'torznab', url: '', api_key: '', categories: '', enabled: true, min_interval_ms: 2000 };
		agg = { url: '', api_key: '' };
		found = null;
		picked = [];
		probe = null;
	}

	function pickSource(next: Source): void {
		source = next;
		probe = null;
	}

	function feedBody(): IndexerInput {
		const body: IndexerInput = {
			name: form.name, protocol: form.protocol, url: form.url, categories: parseCategories(form.categories),
			enabled: form.enabled, min_interval_ms: Number(form.min_interval_ms) || 0
		};
		if (form.api_key.trim()) body.api_key = form.api_key.trim();
		return body;
	}

	async function tryFeed(): Promise<void> {
		if (!form.url.trim()) return;
		probing = true;
		try {
			const body = feedBody();
			// A name is not needed to try a feed.
			if (!body.name?.trim()) delete body.name;
			probe = await api.acquire.probeIndexer(editing ? { ...body, id: editing.id } : body);
		} catch (err) {
			probe = { ok: false, sample: [], error: errorMessage(err, t('acquire.indexers.saveFailed')) };
		} finally {
			probing = false;
		}
	}

	async function saveFeed(): Promise<void> {
		const body = feedBody();
		const saved = editing ? await api.acquire.updateIndexer(editing.id, body) : await api.acquire.createIndexer(body);
		open = false;
		toasts.success(t('acquire.indexers.saved', { name: saved.name }));
		onsaved(editing ? undefined : saved);
	}

	async function discover(): Promise<void> {
		const res = await api.acquire.discoverIndexers({ source: source as AggregatorSource, url: agg.url.trim(), api_key: agg.api_key.trim() });
		found = res.indexers;
		foundFor = askedWith;
		// What the aggregator has switched on and lain does not have yet.
		picked = res.indexers.filter((f) => f.enabled && !f.added_as).map((f) => f.source_id);
		// Enter adds what is ticked; Shift+Tab reaches the boxes.
		focusSoon(() => document.getElementById('indexer-submit'));
	}

	async function importPicked(): Promise<void> {
		const res = await api.acquire.importIndexers({ source: source as AggregatorSource, url: agg.url.trim(), api_key: agg.api_key.trim(), ids: picked });
		open = false;
		toasts.success(t('acquire.indexers.imported', { count: res.added.length, source: sourceLabel(source) }));
		onsaved();
	}

	async function submit(e: SubmitEvent): Promise<void> {
		e.preventDefault();
		saving = true;
		try {
			if (source === 'feed') await saveFeed();
			else if (found === null || stale) await discover();
			else if (picked.length > 0) await importPicked();
		} catch (err) {
			toasts.error(errorMessage(err, source === 'feed' ? t('acquire.indexers.saveFailed') : t('acquire.indexers.discoverFailed', { source: sourceLabel(source) })));
		} finally {
			saving = false;
		}
	}

	function toggle(id: string): void {
		picked = picked.includes(id) ? picked.filter((p) => p !== id) : [...picked, id];
	}

	function onFormKey(e: KeyboardEvent): void {
		if (open && e.altKey && !e.ctrlKey && !e.metaKey && e.code === 'KeyT' && source === 'feed') {
			e.preventDefault();
			void tryFeed();
		}
	}

	function privacyLabel(p: NonNullable<DiscoveredIndexer['privacy']>): string {
		return p === 'public' ? t('acquire.indexers.privacyPublic') : p === 'private' ? t('acquire.indexers.privacyPrivate') : t('acquire.indexers.privacySemi');
	}

	function sourceLabel(s: Source): string {
		return s === 'prowlarr' ? 'Prowlarr' : s === 'jackett' ? 'Jackett' : t('acquire.indexers.sourceFeed');
	}

	const primaryLabel = $derived(
		source === 'feed'
			? t('acquire.indexers.save')
			: found === null || stale
				? t('acquire.indexers.find')
				: t('acquire.indexers.addPicked', { count: picked.length })
	);

	const capsLine = (p: IndexerProbe): string => {
		const c = p.caps;
		if (!c) return '';
		const modes = [c.search && t('acquire.indexers.capSearch'), c.tv_search && t('acquire.indexers.capTv'), c.movie_search && t('acquire.indexers.capMovie'), c.book_search && t('acquire.indexers.capBook')].filter(Boolean) as string[];
		return t('acquire.indexers.probeOk', { modes: modes.join(' · ') || '—', count: c.categories.length });
	};

	const field = 'h-10 w-full rounded-md border border-transparent bg-field px-3 text-sm text-foreground placeholder:text-muted/60 focus:border-accent/60 focus:outline-none';
	const label = 'font-mono text-[10px] uppercase tracking-[0.2em] text-muted';
</script>

<svelte:window onkeydown={onFormKey} />

<Modal bind:open title={editing ? t('acquire.indexers.editTitle', { name: editing.name }) : t('acquire.indexers.addTitle')} description={editing ? t('acquire.indexers.formHint') : t('acquire.indexers.addHint')}>
	<form id="indexer-form" class="space-y-3" onsubmit={submit}>
		{#if !editing}
			<div class="space-y-1.5">
				<span class="block text-xs text-muted">{t('acquire.indexers.source')}</span>
				<Segmented
					label={t('acquire.indexers.source')}
					value={source}
					onchange={pickSource}
					options={[
						{ value: 'prowlarr', label: 'Prowlarr' },
						{ value: 'jackett', label: 'Jackett' },
						{ value: 'feed', label: t('acquire.indexers.sourceFeed') }
					]}
				/>
			</div>
		{/if}

		{#if source !== 'feed'}
			<p class="text-xs text-muted">{source === 'prowlarr' ? t('acquire.indexers.prowlarrHint') : t('acquire.indexers.jackettHint')}</p>
			<label class="block space-y-1.5"><span class="text-xs text-muted">{t('acquire.indexers.baseUrl')}</span>
				<input data-autofocus bind:value={agg.url} type="url" required spellcheck={false} autocomplete="off" class="{field} font-mono text-xs" placeholder={DEFAULT_PORT[source]} />
			</label>
			<label class="block space-y-1.5"><span class="text-xs text-muted">{t('acquire.indexers.apiKey')}</span>
				<input bind:value={agg.api_key} type="password" required autocomplete="off" spellcheck={false} class="{field} font-mono text-xs" />
			</label>
			{#if found !== null}
				<div id="discovered" class="space-y-1.5" class:opacity-50={stale}>
					<p class={label}>{t('acquire.indexers.foundCount', { count: found.length, source: sourceLabel(source) })}</p>
					{#if found.length === 0}
						<p class="text-sm text-muted">{t('acquire.indexers.foundNone', { source: sourceLabel(source) })}</p>
					{:else}
						<ul class="max-h-64 space-y-0.5 overflow-y-auto" role="group" aria-label={t('acquire.indexers.foundCount', { count: found.length, source: sourceLabel(source) })}>
							{#each found as f (f.source_id)}
								<li>
									<label class="flex min-h-9 cursor-pointer items-center gap-3 rounded-md px-2 text-sm hover:bg-surface-active/40 {f.added_as ? 'cursor-default text-muted' : 'text-foreground'}">
										<input type="checkbox" class="accent-current" disabled={!!f.added_as || stale} checked={!!f.added_as || picked.includes(f.source_id)} onchange={() => toggle(f.source_id)} />
										<span class="min-w-0 flex-1 truncate">{f.name}</span>
										<span class="{label} text-[9px]">{f.protocol}</span>
										{#if f.privacy}<span class="{label} hidden text-[9px] sm:inline">{privacyLabel(f.privacy)}</span>{/if}
										{#if f.added_as}<span class="{label} text-[9px] text-accent">{t('acquire.indexers.alreadyAdded')}</span>
										{:else if !f.enabled}<span class="{label} text-[9px] text-warning">{t('acquire.indexers.offThere', { source: sourceLabel(source) })}</span>{/if}
									</label>
								</li>
							{/each}
						</ul>
					{/if}
					{#if stale}<p class="text-xs text-warning">{t('acquire.indexers.findAgain')}</p>{/if}
				</div>
			{/if}
		{:else}
			<label class="block space-y-1.5"><span class="text-xs text-muted">{t('acquire.indexers.name')}</span>
				<input data-autofocus bind:value={form.name} maxlength="60" required class={field} placeholder="tracker-exemplo" />
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
			<div class="space-y-2 pt-1" aria-live="polite">
				<Button type="button" size="field" variant="tonal" loading={probing} disabled={!form.url.trim()} onclick={() => void tryFeed()}>
					{t('acquire.indexers.tryFeed')} <kbd class="ms-1 font-mono text-[10px] text-muted">Alt T</kbd>
				</Button>
				{#if probe}
					{#if probe.ok}
						<p class="text-xs text-accent">{capsLine(probe)}</p>
						{#if probe.sample.length > 0}
							<p class={label}>{t('acquire.indexers.sample')}</p>
							<ul class="space-y-1">
								{#each probe.sample as r, i (i)}
									<li class="flex items-baseline gap-3 text-xs">
										<span class="min-w-0 flex-1 truncate font-mono text-foreground/85" title={r.title}>{r.title}</span>
										<span class="shrink-0 font-mono text-[10px] text-muted">{formatBytes(r.size)}{#if r.protocol === 'torrent'} · {t('acquire.indexers.seeders', { count: r.seeders })}{/if}</span>
									</li>
								{/each}
							</ul>
						{:else}
							<p class="text-xs text-muted">{probe.error ? t('acquire.indexers.sampleFailed', { error: probe.error }) : t('acquire.indexers.sampleEmpty')}</p>
						{/if}
					{:else}
						<p class="text-xs text-danger">{probe.error}</p>
					{/if}
				{/if}
				{#if !editing}<p class="text-xs text-muted">{t('acquire.indexers.noPublic')}</p>{/if}
			</div>
		{/if}
	</form>
	{#snippet footer()}
		<Button variant="ghost" onclick={() => (open = false)}>{t('common.cancel')} <kbd class="ms-1 font-mono text-[10px] text-muted">Esc</kbd></Button>
		<Button id="indexer-submit" type="submit" form="indexer-form" loading={saving} disabled={source !== 'feed' && found !== null && !stale && picked.length === 0}>
			{primaryLabel} <kbd class="ms-1 font-mono text-[10px] opacity-70">Enter</kbd>
		</Button>
	{/snippet}
</Modal>
