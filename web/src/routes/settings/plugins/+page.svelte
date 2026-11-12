<script lang="ts">
	import { onMount } from 'svelte';
	import ArrowDown from '@lucide/svelte/icons/arrow-down';
	import ArrowUp from '@lucide/svelte/icons/arrow-up';
	import Plus from '@lucide/svelte/icons/plus';
	import X from '@lucide/svelte/icons/x';
	import type { BindingView, PluginsInfo, ProviderInfo } from '$lib/api/types';
	import { api, ApiError } from '$lib/api';
	import Badge from '$lib/components/primitives/Badge.svelte';
	import Button from '$lib/components/primitives/Button.svelte';
	import ErrorState from '$lib/components/primitives/ErrorState.svelte';
	import Modal from '$lib/components/primitives/Modal.svelte';
	import Skeleton from '$lib/components/primitives/Skeleton.svelte';
	import SettingsGroup from '$lib/components/settings/SettingsGroup.svelte';
	import { t, type Key } from '$lib/i18n';
	import { toasts } from '$lib/stores/toasts.svelte';
	import { errorMessage } from '$lib/utilities/errors';
	import { formatRelative } from '$lib/utilities/format';

	let info = $state<PluginsInfo | null>(null);
	let loading = $state(true);
	let error = $state<string | null>(null);

	let swapTarget = $state<BindingView | null>(null);
	let swapOpen = $state(false);
	let selected = $state<string[]>([]);
	let swapping = $state(false);
	let swapError = $state<string | null>(null);

	// After a swap (`quiet`) the table refreshes in place: the skeleton swapped
	// it out, so the focus fell to the page and the place in the list was lost.
	async function load(quiet = false): Promise<void> {
		if (!quiet) {
			loading = true;
			error = null;
		}
		try {
			info = await api.plugins.info();
		} catch (err) {
			if (quiet) toasts.error(errorMessage(err));
			else error = errorMessage(err);
		} finally {
			loading = false;
		}
	}

	onMount(() => {
		void load();
	});

	function openSwap(binding: BindingView): void {
		swapTarget = binding;
		// A component bound before it is installed is not offered again: the
		// server refuses a provider it does not know, so a swap that kept it
		// could never be applied.
		selected = binding.providers.filter((id) => !!providerInfo(id));
		swapError = null;
		swapOpen = true;
	}

	const candidateList = $derived(
		swapTarget
			? (info?.provider_info ?? []).filter((p) => p.capabilities.includes(swapTarget!.capability))
			: []
	);
	// Swaps, fallbacks and withdrawals: what the group is for. Every
	// provider registering at start is one line of a count, not twenty rows.
	const events = $derived((info?.events ?? []).filter((e) => e.kind !== 'register'));
	const registered = $derived((info?.events ?? []).filter((e) => e.kind === 'register').length);

	/*
	 * The jobs in families, so a list of thirty-odd reads as a few groups:
	 * the family is the capability's first word after lain., mapped to a
	 * heading; an unknown family (a component's) goes under Other.
	 */
	const FAMILIES: [string, Key][] = [
		['library', 'settings.pluginsPage.family.library'],
		['playback', 'settings.pluginsPage.family.playback'],
		['metadata', 'settings.pluginsPage.family.metadata'],
		['acquire', 'settings.pluginsPage.family.acquire'],
		['people', 'settings.pluginsPage.family.people'],
		['server', 'settings.pluginsPage.family.server'],
		['other', 'settings.pluginsPage.family.misc']
	];
	function familyOf(cap: string): string {
		const word = cap.replace(/^lain\./, '').split(/[.@]/)[0];
		const map: Record<string, string> = {
			source: 'library',
			ingest: 'library',
			catalog: 'library',
			media: 'library',
			comic: 'library',
			search: 'library',
			playback: 'playback',
			transform: 'playback',
			metadata: 'metadata',
			acquire: 'acquire',
			indexer: 'acquire',
			release: 'acquire',
			subtitle: 'acquire',
			list: 'people',
			listlink: 'people',
			social: 'people',
			userstate: 'people',
			backup: 'server',
			settings: 'server',
			ui: 'server'
		};
		return map[word] ?? 'other';
	}
	const families = $derived(
		FAMILIES.map(([id, title]) => ({
			id,
			title,
			bindings: (info?.composition ?? []).filter((b) => familyOf(b.capability) === id)
		})).filter((f) => f.bindings.length > 0)
	);
	const availableList = $derived(candidateList.filter((p) => !selected.includes(p.id)));
	const exactlyOne = $derived(swapTarget?.mode === 'exactly-one');
	const canSwap = $derived(exactlyOne ? selected.length === 1 : selected.length >= 1);

	function providerInfo(id: string): ProviderInfo | undefined {
		return info?.provider_info.find((p) => p.id === id);
	}

	function add(id: string): void {
		if (exactlyOne) {
			selected = [id];
			return;
		}
		if (!selected.includes(id)) selected = [...selected, id];
	}

	function removeAt(index: number): void {
		selected = selected.filter((_, i) => i !== index);
	}

	function move(index: number, delta: number): void {
		const next = [...selected];
		const target = index + delta;
		if (target < 0 || target >= next.length) return;
		[next[index], next[target]] = [next[target], next[index]];
		selected = next;
	}

	async function applySwap(): Promise<void> {
		const binding = swapTarget;
		if (!binding || !canSwap) return;
		swapping = true;
		swapError = null;
		try {
			const result = await api.plugins.swap({
				capability: binding.capability,
				providers: selected,
				generation: binding.generation
			});
			if (info) info = { ...info, composition: result.composition };
			swapOpen = false;
			toasts.success(t('settings.pluginsPage.swapped', { capability: binding.capability, n: result.generation }));
			await load(true);
		} catch (err) {
			if (err instanceof ApiError && err.kind === 'conflict') {
				// Stale writer: somebody changed the binding first. Reload
				// the authoritative state instead of retrying blindly.
				swapError = t('settings.pluginsPage.stale', { error: err.message });
				await load(true);
			} else if (err instanceof ApiError && err.kind === 'unavailable') {
				swapError = t('settings.pluginsPage.unhealthyCandidate', { error: err.message });
				await load(true);
			} else {
				swapError = errorMessage(err);
			}
		} finally {
			swapping = false;
		}
	}

	function shortCapability(cap: string): string {
		return cap.replace(/@\d+$/, '');
	}

	/*
	 * A composition can hold two generations of one capability
	 * (lain.playback.transcode@1 and @2). Hiding the version rendered two
	 * identical rows with two Replace buttons and no way to tell which
	 * binding a button acts on, so the version stays on screen.
	 */
	function capabilityVersion(cap: string): string {
		return cap.match(/@\d+$/)?.[0] ?? '';
	}

	/*
	 * The rows are capability ids, binding modes and generation counters:
	 * the machine's own vocabulary. Plain language first, ids second, so an
	 * admin looking for 'where do metadata titles come from' can find it.
	 * The sentence comes from the server, which describes every job it
	 * knows and lets a component describe the ones it brings (D-285); a
	 * server from before that falls back to the few this client knows.
	 */
	const KNOWN: Record<string, Key> = {
		'lain.source.enumerate': 'settings.pluginsPage.purpose.source_enumerate',
		'lain.media.identify': 'settings.pluginsPage.purpose.media_identify',
		'lain.ingest.scan': 'settings.pluginsPage.purpose.ingest_scan',
		'lain.search.query': 'settings.pluginsPage.purpose.search_query',
		'lain.catalog.read': 'settings.pluginsPage.purpose.catalog_read',
		'lain.catalog.write': 'settings.pluginsPage.purpose.catalog_write',
		'lain.userstate.progress': 'settings.pluginsPage.purpose.userstate_progress',
		'lain.media.probe': 'settings.pluginsPage.purpose.media_probe',
		'lain.playback.plan': 'settings.pluginsPage.purpose.playback_plan',
		'lain.playback.transcode': 'settings.pluginsPage.purpose.playback_transcode',
		'lain.metadata.search': 'settings.pluginsPage.purpose.metadata_search',
		'lain.metadata.resolve': 'settings.pluginsPage.purpose.metadata_resolve',
		'lain.transform.thumbnail': 'settings.pluginsPage.purpose.transform_thumbnail'
	};

	function purposeOf(binding: BindingView): string {
		if (binding.summary) return binding.summary;
		const key = KNOWN[shortCapability(binding.capability)];
		return key ? t(key) : '';
	}

	function modeTone(mode: string): 'neutral' | 'accent' | 'warning' {
		if (mode === 'exactly-one') return 'accent';
		if (mode === 'fan-out') return 'warning';
		return 'neutral';
	}
</script>

<svelte:head><title>{t('settings.pluginsPage.pageTitle')}</title></svelte:head>

<div>
	{#if loading}
		<div class="space-y-3">
			{#each Array(4) as _}
				<Skeleton class="h-20" />
			{/each}
		</div>
	{:else if error}
		<ErrorState message={error} retry={() => void load()} />
	{:else if info}
		<SettingsGroup id="capabilities" title={t('settings.pluginsPage.capabilities')} description={t('settings.groups.plugins.capabilities')}>
			<p class="settings-note">{t('settings.pluginsPage.note')}</p>
			<!-- A table: the job, who does it, how they are bound, and the swap.
			     Read across one line per job on a desktop; stacked on a phone. -->
			<ul class="settings-items" aria-label={t('settings.pluginsPage.capabilities')}>
				<li class="hidden gap-x-6 py-2.5 font-mono text-[10px] uppercase tracking-[0.18em] text-muted lg:grid lg:grid-cols-[minmax(0,1.1fr)_minmax(0,1fr)_7.5rem_5.5rem]" aria-hidden="true">
					<span>{t('settings.groups.plugins.colJob')}</span>
					<span>{t('settings.groups.plugins.colProviders')}</span>
					<span>{t('settings.groups.plugins.colMode')}</span>
					<span></span>
				</li>
				{#each families as family (family.id)}
				<li class="pb-2 pt-5 font-mono text-[10px] uppercase tracking-[0.2em] text-muted first:pt-3" role="presentation">{t(family.title)}</li>
				{#each family.bindings as binding (binding.capability)}
					{@const installed = (info.provider_info ?? []).some((p) => p.capabilities.includes(binding.capability))}
					{@const purpose = purposeOf(binding)}
					{@const missing = binding.providers.filter((id) => !providerInfo(id))}
					<li class="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-x-6 gap-y-2 py-3 lg:grid-cols-[minmax(0,1.1fr)_minmax(0,1fr)_7.5rem_5.5rem]">
						<div class="min-w-0">
							<!-- Wrapped, never clamped: a sentence cut at "item p…" said less than the id. -->
							{#if purpose}
								<p class="text-sm leading-5 text-foreground">{purpose}</p>
							{/if}
							<p class="truncate font-mono text-xs {purpose ? 'text-muted' : 'text-foreground'}">
								{shortCapability(binding.capability)}<span>{capabilityVersion(binding.capability)}</span>
							</p>
						</div>
						<div class="col-span-2 row-start-2 flex min-w-0 flex-wrap items-center gap-1.5 lg:col-span-1 lg:row-start-auto">
							{#each binding.providers as id, index (id + index)}
								{@const provider = providerInfo(id)}
								<span class="inline-flex max-w-full items-center gap-2 rounded-full bg-field px-2.5 py-1 font-mono text-[11px] text-foreground">
									{#if !exactlyOne && binding.providers.length > 1}
										<span class="text-muted">{index + 1}</span>
									{/if}
									<span class="truncate">{id}</span>
									{#if provider}
										<span
											class={['size-1.5 shrink-0 rounded-full', provider.healthy ? 'bg-success' : 'bg-danger'].join(' ')}
											role="img"
																aria-label={provider.healthy ? t('settings.pluginsPage.healthy') : t('settings.pluginsPage.unhealthy')}
															></span>
														{:else}
															<!-- Bound ahead of its install (a component): named, and said not to be here yet. -->
															<span class="shrink-0 font-sans text-muted">{t('settings.pluginsPage.notInstalled')}</span>
														{/if}
													</span>
												{/each}
												{#if missing.length > 0}
													<!-- What to do about it, on the server: the bridges install as components. -->
													<p class="basis-full text-xs leading-5 text-muted">
														{t('settings.pluginsPage.installHint')} <code class="font-mono text-[11px] text-foreground">lain component install {missing[0]}</code>
													</p>
												{/if}
											</div>
						<div class="col-span-2 row-start-3 flex items-center gap-2 lg:col-span-1 lg:row-start-auto lg:flex-col lg:items-start lg:gap-1">
							<Badge tone={modeTone(binding.mode)}>{binding.mode}</Badge>
							<span class="text-xs text-muted">{t('settings.pluginsPage.generation', { n: binding.generation })}</span>
						</div>
						<!-- Nothing installed can do this job yet: there is nothing to replace with. -->
						<!-- Ghost: thirty filled buttons down one column outweighed the jobs. -->
						<Button
							variant="ghost"
							size="field"
							class="col-start-2 row-start-1 justify-self-end lg:col-start-auto lg:row-start-auto"
							disabled={!installed}
								title={installed ? undefined : t('settings.pluginsPage.replaceNone')}
								onclick={() => openSwap(binding)}
								aria-label={t('settings.pluginsPage.replaceFor', { capability: binding.capability })}
							>
								{t('settings.pluginsPage.replace')}
							</Button>
					</li>
				{/each}
				{/each}
			</ul>
		</SettingsGroup>

		<SettingsGroup id="events" title={t('settings.pluginsPage.events')} description={t('settings.groups.plugins.events')}>
			{#if events.length === 0}
				<p class="py-4 text-sm text-muted">
					{t('settings.pluginsPage.noEvents')}
					{#if registered > 0}{t('settings.pluginsPage.registered', { count: registered })}{/if}
				</p>
			{:else}
				<ul class="settings-items">
					{#each [...events].reverse().slice(0, 20) as event, index (event.at + '-' + index)}
						<li class="flex flex-wrap items-baseline gap-x-3 gap-y-1 py-2.5 text-sm">
							<Badge
								tone={event.kind.includes('rejected') || event.kind.includes('degraded')
									? 'danger'
									: event.kind === 'fallback'
										? 'warning'
										: 'neutral'}
							>
								{event.kind}
							</Badge>
							<span class="font-mono text-xs text-muted">{shortCapability(event.capability ?? '')}</span>
							{#if event.provider}<span class="text-foreground">{event.provider}</span>{/if}
							{#if event.detail}<span class="text-xs text-muted">{event.detail}</span>{/if}
							<span class="ml-auto text-xs text-muted">{formatRelative(event.at)}</span>
						</li>
					{/each}
				</ul>
			{/if}
		</SettingsGroup>
	{/if}
</div>

<!-- Swap -->
<Modal
	bind:open={swapOpen}
	title={t('settings.pluginsPage.swapTitle')}
	description={swapTarget
		? `${swapTarget.capability} · ${swapTarget.mode} · ${t('settings.pluginsPage.generation', { n: swapTarget.generation })}`
		: ''}
>
	{#if swapTarget}
		<div class="space-y-5">
			{#if swapError}
				<div
					class="rounded-md border border-danger/30 bg-danger/10 px-3 py-2 text-sm text-danger"
					role="alert"
				>
					{swapError}
				</div>
			{/if}

			<div>
				<p class="mb-2 text-sm font-medium text-muted">
					{exactlyOne ? t('settings.pluginsPage.pickOne') : t('settings.pluginsPage.order')}
				</p>
				{#if selected.length === 0}
					<p class="rounded-md border border-dashed border-line px-3 py-4 text-center text-sm text-muted">
						{t('settings.pluginsPage.pickAtLeast')}
					</p>
				{:else}
					<ul class="space-y-1.5">
						{#each selected as id, index (id)}
							{@const provider = providerInfo(id)}
							<li
								class="flex items-center gap-2 rounded-md border border-line bg-surface px-3 py-2"
							>
								<span class="w-5 text-center font-mono text-xs text-muted">{index + 1}</span>
								<span class="min-w-0 flex-1 truncate font-mono text-sm text-foreground">{id}</span>
								{#if provider}
									<span
										class={['size-1.5 rounded-full', provider.healthy ? 'bg-success' : 'bg-danger'].join(
											' '
										)}
										role="img"
										aria-label={provider.healthy ? t('settings.pluginsPage.healthy') : t('settings.pluginsPage.unhealthy')}
									></span>
								{/if}
								{#if !exactlyOne}
									<button
										class="rounded p-1 text-muted hover:bg-surface-hover hover:text-foreground disabled:opacity-30"
										onclick={() => move(index, -1)}
										disabled={index === 0}
										aria-label={t('settings.pluginsPage.moveUp', { id })}
									>
										<ArrowUp class="size-3.5" />
									</button>
									<button
										class="rounded p-1 text-muted hover:bg-surface-hover hover:text-foreground disabled:opacity-30"
										onclick={() => move(index, 1)}
										disabled={index === selected.length - 1}
										aria-label={t('settings.pluginsPage.moveDown', { id })}
									>
										<ArrowDown class="size-3.5" />
									</button>
								{/if}
								<button
									class="rounded p-1 text-muted hover:bg-danger/10 hover:text-danger"
									onclick={() => removeAt(index)}
									aria-label={t('settings.pluginsPage.removeProvider', { id })}
								>
									<X class="size-3.5" />
								</button>
							</li>
						{/each}
					</ul>
				{/if}
			</div>

			<div>
				<p class="mb-2 text-sm font-medium text-muted">{t('settings.pluginsPage.available')}</p>
				{#if availableList.length === 0}
					<p class="text-sm text-muted">{t('settings.pluginsPage.allPicked')}</p>
				{:else}
					<ul class="space-y-1.5">
						{#each availableList as provider (provider.id)}
							<li
								class="flex items-center gap-2 rounded-md border border-line/70 bg-surface/50 px-3 py-2"
							>
								<span class="min-w-0 flex-1 truncate font-mono text-sm text-foreground">
									{provider.id}
								</span>
								<span
									class={['size-1.5 rounded-full', provider.healthy ? 'bg-success' : 'bg-danger'].join(
										' '
									)}
									role="img"
									aria-label={provider.healthy ? t('settings.pluginsPage.healthy') : t('settings.pluginsPage.unhealthy')}
								></span>
								<button
									class="rounded p-1 text-muted hover:bg-surface-hover hover:text-accent"
									onclick={() => add(provider.id)}
									aria-label={t('settings.pluginsPage.addProvider', { id: provider.id })}
								>
									<Plus class="size-3.5" />
								</button>
							</li>
						{/each}
					</ul>
				{/if}
			</div>
		</div>
	{/if}

	{#snippet footer()}
		<Button variant="ghost" onclick={() => (swapOpen = false)}>{t('common.cancel')}</Button>
		<Button
			loading={swapping}
			disabled={!canSwap}
			onclick={() => void applySwap()}
		>
			{t('settings.pluginsPage.apply', { n: swapTarget ? swapTarget.generation + 1 : 1 })}
		</Button>
	{/snippet}
</Modal>
