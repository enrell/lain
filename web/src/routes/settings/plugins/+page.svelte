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
	import { t } from '$lib/i18n';
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
	const events = $derived(info?.events ?? []);
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
			toasts.success(`${binding.capability} now generation ${result.generation}.`);
			await load(true);
		} catch (err) {
			if (err instanceof ApiError && err.kind === 'conflict') {
				// Stale writer: somebody changed the binding first. Reload
				// the authoritative state instead of retrying blindly.
				swapError = `${err.message}. Reloading the current composition.`;
				await load(true);
			} else if (err instanceof ApiError && err.kind === 'unavailable') {
				swapError = `${err.message}. The candidate failed its health check; the previous generation keeps serving.`;
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
	 * Unknown capabilities fall back to the id alone.
	 */
	const PURPOSES: Record<string, string> = {
		'lain.source.enumerate': 'Reads the library folders and lists the video files it finds.',
		'lain.media.identify': 'Turns a filename and path into a title, season and episode.',
		'lain.ingest.scan': 'Walks every library root and updates the catalog.',
		'lain.search.query': 'Answers searches over this server\u2019s catalog.',
		'lain.catalog.read': 'Serves the catalog to the app: lists, rails and item pages.',
		'lain.catalog.write': 'Stores items in the catalog and retires the ones that are gone.',
		'lain.userstate.progress': 'Remembers where you stopped watching each item.',
		'lain.media.probe': 'Reads technical stream details: codecs, tracks, dimensions.',
		'lain.playback.plan': 'Decides whether a file plays directly or has to be converted.',
		'lain.playback.transcode': 'Converts a file the browser cannot play and serves the stream.',
		'lain.metadata.search': 'Searches an external metadata source for a matching title.',
		'lain.metadata.resolve': 'Fetches the posters, synopsis and year shown on item pages.',
		'lain.transform.thumbnail': 'Extracts the still frames used as thumbnails and placeholders.'
	};

	function purposeOf(cap: string): string {
		return PURPOSES[shortCapability(cap)] ?? '';
	}

	function modeTone(mode: string): 'neutral' | 'accent' | 'warning' {
		if (mode === 'exactly-one') return 'accent';
		if (mode === 'fan-out') return 'warning';
		return 'neutral';
	}
</script>

<svelte:head><title>Plugins — Settings — Lain</title></svelte:head>

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
		<SettingsGroup id="capabilities" title="Capabilities" description={t('settings.groups.plugins.capabilities')}>
			<p class="settings-note">
				Each row is one job this server does &mdash; reading folders, identifying files, storing items,
				serving playback &mdash; and the plugin currently doing it. Replacing a plugin is safe: the
				change is refused if the new plugin is unhealthy, and a plugin that fails later falls back to
				the one that worked. Only replace a row if you know what the job is for; the id underneath is
				the technical name.
			</p>
			<!-- A table: the job, who does it, how they are bound, and the swap.
			     Read across one line per job on a desktop; stacked on a phone. -->
			<ul class="settings-items" aria-label="Capabilities">
				<li class="hidden gap-x-6 py-2.5 font-mono text-[10px] uppercase tracking-[0.18em] text-muted lg:grid lg:grid-cols-[minmax(0,1.1fr)_minmax(0,1fr)_7.5rem_5.5rem]" aria-hidden="true">
					<span>{t('settings.groups.plugins.colJob')}</span>
					<span>{t('settings.groups.plugins.colProviders')}</span>
					<span>{t('settings.groups.plugins.colMode')}</span>
					<span></span>
				</li>
				{#each info.composition as binding (binding.capability)}
					{@const installed = (info.provider_info ?? []).some((p) => p.capabilities.includes(binding.capability))}
					{@const purpose = purposeOf(binding.capability)}
					<li class="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-x-6 gap-y-2 py-3 lg:grid-cols-[minmax(0,1.1fr)_minmax(0,1fr)_7.5rem_5.5rem]">
						<div class="min-w-0">
							{#if purpose}
								<p class="text-sm text-foreground lg:truncate" title={purpose}>{purpose}</p>
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
											aria-label={provider.healthy ? 'healthy' : 'unhealthy'}
										></span>
									{:else}
										<!-- Bound ahead of its install (a component): named, and said not to be here yet. -->
										<span class="shrink-0 font-sans text-muted">not installed</span>
									{/if}
								</span>
							{/each}
						</div>
						<div class="col-span-2 row-start-3 flex items-center gap-2 lg:col-span-1 lg:row-start-auto lg:flex-col lg:items-start lg:gap-1">
							<Badge tone={modeTone(binding.mode)}>{binding.mode}</Badge>
							<span class="text-xs text-muted">generation {binding.generation}</span>
						</div>
						<!-- Nothing installed can do this job yet: there is nothing to replace with. -->
						<Button
							variant="tonal"
							size="field"
							class="col-start-2 row-start-1 justify-self-end lg:col-start-auto lg:row-start-auto"
							disabled={!installed}
							title={installed ? undefined : 'Install a component that provides this first'}
							onclick={() => openSwap(binding)}
							aria-label={`Replace providers for ${binding.capability}`}
						>
							Replace
						</Button>
					</li>
				{/each}
			</ul>
		</SettingsGroup>

		<SettingsGroup id="events" title="Recent composition events" description={t('settings.groups.plugins.events')}>
			{#if events.length === 0}
				<p class="py-4 text-sm text-muted">No swaps or fallbacks recorded in this process.</p>
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
	title="Replace providers"
	description={swapTarget
		? `${swapTarget.capability} · ${swapTarget.mode} · generation ${swapTarget.generation}`
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
					{exactlyOne ? 'Choose the single provider' : 'Serving order'}
				</p>
				{#if selected.length === 0}
					<p class="rounded-md border border-dashed border-line px-3 py-4 text-center text-sm text-muted">
						Pick at least one provider below.
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
										aria-label={provider.healthy ? 'healthy' : 'unhealthy'}
									></span>
								{/if}
								{#if !exactlyOne}
									<button
										class="rounded p-1 text-muted hover:bg-surface-hover hover:text-foreground disabled:opacity-30"
										onclick={() => move(index, -1)}
										disabled={index === 0}
										aria-label={`Move ${id} up`}
									>
										<ArrowUp class="size-3.5" />
									</button>
									<button
										class="rounded p-1 text-muted hover:bg-surface-hover hover:text-foreground disabled:opacity-30"
										onclick={() => move(index, 1)}
										disabled={index === selected.length - 1}
										aria-label={`Move ${id} down`}
									>
										<ArrowDown class="size-3.5" />
									</button>
								{/if}
								<button
									class="rounded p-1 text-muted hover:bg-danger/10 hover:text-danger"
									onclick={() => removeAt(index)}
									aria-label={`Remove ${id}`}
								>
									<X class="size-3.5" />
								</button>
							</li>
						{/each}
					</ul>
				{/if}
			</div>

			<div>
				<p class="mb-2 text-sm font-medium text-muted">Available for this capability</p>
				{#if availableList.length === 0}
					<p class="text-sm text-muted">Every registered candidate is already selected.</p>
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
									aria-label={provider.healthy ? 'healthy' : 'unhealthy'}
								></span>
								<button
									class="rounded p-1 text-muted hover:bg-surface-hover hover:text-accent"
									onclick={() => add(provider.id)}
									aria-label={`Add ${provider.id}`}
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
		<Button variant="ghost" onclick={() => (swapOpen = false)}>Cancel</Button>
		<Button
			loading={swapping}
			disabled={!canSwap}
			onclick={() => void applySwap()}
		>
			Apply generation {swapTarget ? swapTarget.generation + 1 : ''}
		</Button>
	{/snippet}
</Modal>
