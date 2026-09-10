<script lang="ts">
	import { onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import ArrowLeft from '@lucide/svelte/icons/arrow-left';
	import MonitorPlay from '@lucide/svelte/icons/monitor-play';
	import type { CatalogItem, PlaybackPlan, Progress } from '$lib/api/types';
	import { api, ApiError } from '$lib/api';
	import Player from '$lib/components/player/Player.svelte';
	import Button from '$lib/components/primitives/Button.svelte';
	import ErrorState from '$lib/components/primitives/ErrorState.svelte';
	import Spinner from '$lib/components/primitives/Spinner.svelte';
	import { errorMessage } from '$lib/utilities/errors';
	import { itemCache } from '$lib/stores/media-cache.svelte';

	const id = $derived(page.params.id ?? '');

	let item = $state<CatalogItem | null>(null);
	let plan = $state<PlaybackPlan | null>(null);
	let progress = $state<Progress | null>(null);
	let loading = $state(true);
	let error = $state<string | null>(null);
	let notFound = $state(false);
	const detailHref = $derived(item ? `/item/${item.id}` : '/library');

	async function load(): Promise<void> {
		loading = true;
		error = null;
		notFound = false;
		try {
			const [it, pl, prog] = await Promise.all([
				api.catalog.get(id),
				api.playback.plan(id),
				api.progress.get(id)
			]);
			item = it;
			itemCache.set(it.id, it);
			plan = pl;
			progress = prog;
		} catch (err) {
			if (err instanceof ApiError && err.kind === 'not-found') notFound = true;
			else error = errorMessage(err);
		} finally {
			loading = false;
		}
	}

	onMount(() => {
		void load();
	});
</script>

<svelte:head><title>{item?.title ? `Playing ${item.title} — Lain` : 'Player — Lain'}</title></svelte:head>

{#if loading}
	<div class="flex h-dvh items-center justify-center bg-black">
		<Spinner class="size-8 text-white/70" label="Preparing playback" />
	</div>
{:else if notFound}
	<div class="flex h-dvh flex-col items-center justify-center gap-4 bg-black px-6 text-center">
		<p class="text-sm text-muted">That item no longer exists in the catalog.</p>
		<Button variant="secondary" onclick={() => void goto('/library')}>
			<ArrowLeft class="size-4" /> Back to library
		</Button>
	</div>
{:else if error}
	<div class="flex h-dvh items-center justify-center bg-black px-6">
		<ErrorState message={error} retry={() => void load()} />
	</div>
{:else if item && plan}
	{#if plan.available}
		<Player {item} {plan} initialProgress={progress} />
	{:else}
		<div class="relative flex h-dvh items-center justify-center overflow-hidden bg-black px-6">
			<div class="lattice absolute inset-0 opacity-30"></div>
			<div class="relative max-w-lg text-center">
				<MonitorPlay class="mx-auto size-10 text-muted" />
				<h1 class="mt-5 text-xl font-semibold text-foreground">{item.title}</h1>
				<p class="mt-3 text-sm leading-relaxed text-muted">
					{plan.reason ?? 'No playback plan is available for this file in the browser.'}
				</p>
				<p class="mt-3 text-xs leading-relaxed text-muted/80">
					The server only sends bytes it can actually serve: no transcoder is installed yet, so
					the browser cannot be given this container. The desktop/CLI client plays it directly.
				</p>
				<div class="mt-7 flex flex-wrap items-center justify-center gap-2">
					<Button variant="secondary" onclick={() => void goto(detailHref)}>
						<ArrowLeft class="size-4" /> Back to details
					</Button>
					<Button variant="ghost" onclick={() => void goto('/library')}>Browse library</Button>
				</div>
			</div>
		</div>
	{/if}
{/if}
