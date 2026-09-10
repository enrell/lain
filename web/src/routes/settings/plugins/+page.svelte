<script lang="ts">
	import { onMount } from 'svelte';
	import Activity from '@lucide/svelte/icons/activity';
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

	async function load(): Promise<void> {
		loading = true;
		error = null;
		try {
			info = await api.plugins.info();
		} catch (err) {
			error = errorMessage(err);
		} finally {
			loading = false;
		}
	}

	onMount(() => {
		void load();
	});

	function openSwap(binding: BindingView): void {
		swapTarget = binding;
		selected = [...binding.providers];
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
			await load();
		} catch (err) {
			if (err instanceof ApiError && err.kind === 'conflict') {
				// Stale writer: somebody changed the binding first. Reload
				// the authoritative state instead of retrying blindly.
				swapError = `${err.message}. Reloading the current composition.`;
				await load();
			} else if (err instanceof ApiError && err.kind === 'unavailable') {
				swapError = `${err.message}. The candidate failed its health check; the previous generation keeps serving.`;
				await load();
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

	function modeTone(mode: string): 'neutral' | 'accent' | 'warning' {
		if (mode === 'exactly-one') return 'accent';
		if (mode === 'fan-out') return 'warning';
		return 'neutral';
	}
</script>

<svelte:head><title>Plugins — Settings — Lain</title></svelte:head>

<div class="space-y-6">
	<p class="max-w-3xl text-sm leading-relaxed text-muted">
		The composition decides which provider serves each capability. Replacing one is fenced by
		generation: a stale write is rejected, and an unhealthy candidate is refused before the active
		generation changes. A provider that fails at call time falls back to the last good one.
	</p>

	{#if loading}
		<div class="space-y-3">
			{#each Array(4) as _}
				<Skeleton class="h-24" />
			{/each}
		</div>
	{:else if error}
		<ErrorState message={error} retry={() => void load()} />
	{:else if info}
		<section class="space-y-3">
			<h2 class="text-base font-semibold text-foreground">Capabilities</h2>
			<ul class="space-y-3">
				{#each info.composition as binding (binding.capability)}
					<li class="rounded-card border border-line bg-surface/50 p-4">
						<div class="flex flex-wrap items-center justify-between gap-3">
							<div class="min-w-0">
								<p class="truncate font-mono text-sm text-foreground">
									{shortCapability(binding.capability)}
								</p>
								<div class="mt-1.5 flex flex-wrap items-center gap-2">
									<Badge tone={modeTone(binding.mode)}>{binding.mode}</Badge>
									<span class="text-xs text-muted">generation {binding.generation}</span>
								</div>
							</div>
							<Button
								variant="secondary"
								size="sm"
								onclick={() => openSwap(binding)}
								aria-label={`Replace providers for ${binding.capability}`}
							>
								Replace
							</Button>
						</div>
						<div class="mt-3 flex flex-wrap items-center gap-2">
							{#each binding.providers as id, index (id + index)}
								{@const provider = providerInfo(id)}
								<span
									class="inline-flex items-center gap-2 rounded-full border border-line bg-surface px-3 py-1 font-mono text-xs text-foreground"
								>
									{#if !exactlyOne && binding.providers.length > 1}
										<span class="text-muted">{index + 1}</span>
									{/if}
									{id}
									{#if provider}
										<span
											class={[
												'size-1.5 rounded-full',
												provider.healthy ? 'bg-success' : 'bg-danger'
											].join(' ')}
											role="img"
											aria-label={provider.healthy ? 'healthy' : 'unhealthy'}
										></span>
									{/if}
								</span>
							{/each}
						</div>
					</li>
				{/each}
			</ul>
		</section>

		<section class="space-y-3">
			<h2 class="flex items-center gap-2 text-base font-semibold text-foreground">
				<Activity class="size-4 text-muted" /> Recent composition events
			</h2>
			{#if events.length === 0}
				<p class="text-sm text-muted">No swaps or fallbacks recorded in this process.</p>
			{:else}
				<ul class="divide-y divide-line overflow-hidden rounded-card border border-line">
					{#each [...events].reverse().slice(0, 20) as event, index (event.at + '-' + index)}
						<li class="flex flex-wrap items-baseline gap-x-3 gap-y-1 bg-surface/30 px-4 py-2.5 text-sm">
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
		</section>
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
