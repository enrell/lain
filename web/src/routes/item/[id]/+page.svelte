<script lang="ts">
	import { onMount } from 'svelte';
	import { page } from '$app/state';
	import ArrowLeft from '@lucide/svelte/icons/arrow-left';
	import Clapperboard from '@lucide/svelte/icons/clapperboard';
	import Play from '@lucide/svelte/icons/play';
	import RotateCcw from '@lucide/svelte/icons/rotate-ccw';
	import Sparkles from '@lucide/svelte/icons/sparkles';
	import Trash2 from '@lucide/svelte/icons/trash-2';
	import type { CatalogItem, Enrichment, Library, PlaybackPlan, Progress } from '$lib/api/types';
	import { api, ApiError } from '$lib/api';
	import { session } from '$lib/auth/session.svelte';
	import Badge from '$lib/components/primitives/Badge.svelte';
	import Button from '$lib/components/primitives/Button.svelte';
	import EmptyState from '$lib/components/primitives/EmptyState.svelte';
	import ErrorState from '$lib/components/primitives/ErrorState.svelte';
	import LinkButton from '$lib/components/primitives/LinkButton.svelte';
	import Modal from '$lib/components/primitives/Modal.svelte';
	import Poster from '$lib/components/media/Poster.svelte';
	import ProgressBar from '$lib/components/media/ProgressBar.svelte';
	import Skeleton from '$lib/components/primitives/Skeleton.svelte';
	import { ensureLibraries, applyEnrichment, itemCache } from '$lib/stores/media-cache.svelte';
	import { toasts } from '$lib/stores/toasts.svelte';
	import { errorMessage } from '$lib/utilities/errors';
	import { formatBytes, formatDate, formatRelative, formatTime, mediaSubtitle } from '$lib/utilities/format';
	import { progressRatio } from '$lib/utilities/progress';

	const id = $derived(page.params.id ?? '');

	let item = $state<CatalogItem | null>(null);
	let enrichment = $state<Enrichment | null>(null);
	let progress = $state<Progress | null>(null);
	let plan = $state<PlaybackPlan | null>(null);
	let library = $state<Library | undefined>(undefined);
	let loading = $state(true);
	let error = $state<string | null>(null);
	let notFound = $state(false);

	let enriching = $state(false);
	let resetting = $state(false);
	let confirmRemove = $state(false);
	let removing = $state(false);
	let backdropFailedId = $state<string | null>(null);

	// Cover, else poster, else a wide still extracted by the server.
	const backdropSrc = $derived(
		enrichment?.cover ||
			enrichment?.poster ||
			(item ? api.thumbnail.url(item.id, session.token, { at: 30, width: 960 }) : '')
	);

	async function load(): Promise<void> {
		loading = true;
		error = null;
		notFound = false;
		try {
			const [it, prog, pl, enr, libs] = await Promise.all([
				api.catalog.get(id),
				api.progress.get(id),
				api.playback.plan(id),
				api.enrich.get(id),
				ensureLibraries()
			]);
			item = it;
			itemCache.set(it.id, it);
			progress = prog;
			plan = pl;
			enrichment = enr;
			if (enr) applyEnrichment(enr);
			library = libs.find((lib) => lib.id === it.library_id);
		} catch (err) {
			if (err instanceof ApiError && err.kind === 'not-found') {
				notFound = true;
			} else {
				error = errorMessage(err);
			}
		} finally {
			loading = false;
		}
	}

	onMount(() => {
		void load();
	});

	const playable = $derived(plan?.available === true);
	const resumeAt = $derived(
		progress && !progress.completed && progress.position_sec >= 5 ? progress.position_sec : 0
	);
	const title = $derived(enrichment?.title || item?.title || '');
	const ratio = $derived(progress ? progressRatio(progress) : 0);

	async function fetchMetadata(): Promise<void> {
		if (!item || enriching) return;
		enriching = true;
		try {
			const result = await api.enrich.run(item.id);
			enrichment = result;
			applyEnrichment(result);
			toasts.success(`Metadata added from ${result.provider}.`);
		} catch (err) {
			toasts.error(errorMessage(err, 'Could not fetch metadata.'));
		} finally {
			enriching = false;
		}
	}

	async function removeMetadata(): Promise<void> {
		if (!item) return;
		removing = true;
		try {
			await api.enrich.remove(item.id);
			enrichment = null;
			confirmRemove = false;
			toasts.success('Metadata overlay removed.');
		} catch (err) {
			toasts.error(errorMessage(err, 'Could not remove the overlay.'));
		} finally {
			removing = false;
		}
	}

	async function resetProgress(): Promise<void> {
		if (!item || resetting) return;
		resetting = true;
		try {
			progress = await api.progress.put(item.id, {
				position_sec: 0,
				duration_sec: progress?.duration_sec ?? 0,
				completed: false
			});
			toasts.success('Progress reset.');
		} catch (err) {
			toasts.error(errorMessage(err, 'Could not reset progress.'));
		} finally {
			resetting = false;
		}
	}
</script>

<svelte:head><title>{title || 'Item'} — Lain</title></svelte:head>

{#if loading}
	<div class="space-y-6">
		<Skeleton class="h-56 w-full" />
		<div class="flex gap-6">
			<Skeleton class="h-72 w-48 shrink-0" />
			<div class="flex-1 space-y-3">
				<Skeleton class="h-8 w-2/3" />
				<Skeleton class="h-4 w-1/3" />
				<Skeleton class="h-24 w-full" />
			</div>
		</div>
	</div>
{:else if notFound}
	<EmptyState
		title="That item is gone"
		description="It was removed from the catalog, likely by a scan or library deletion."
	>
		{#snippet icon()}<Clapperboard class="size-6 text-muted" />{/snippet}
		<LinkButton href="/library" variant="secondary">
			<ArrowLeft class="size-4" /> Back to library
		</LinkButton>
	</EmptyState>
{:else if error}
	<ErrorState message={error} retry={() => void load()} />
{:else if item}
	<article class="space-y-8">
		<!-- Artwork backdrop: cover, else poster, else a generated still. -->
		<div class="relative -mx-4 -mt-5 h-44 overflow-hidden md:-mx-8 md:-mt-8 md:h-60">
			{#if item && backdropSrc && backdropFailedId !== item.id}
				<img
					src={backdropSrc}
					alt=""
					class="size-full object-cover opacity-40"
					referrerpolicy="no-referrer"
					onerror={() => (backdropFailedId = item?.id ?? null)}
				/>
			{:else}
				<div class="lattice size-full opacity-50"></div>
			{/if}
			<div class="absolute inset-0 bg-gradient-to-t from-background via-background/70 to-transparent"></div>
		</div>

		<div class="relative -mt-28 flex flex-col gap-6 md:-mt-36 md:flex-row md:gap-8">
			<div class="w-36 shrink-0 md:w-52">
				<Poster {item} {enrichment} class="aspect-[2/3] rounded-card border border-line shadow-2xl" priority />
			</div>

			<div class="min-w-0 flex-1 space-y-4 pt-1 md:pt-16">
				<div>
					<h1 class="text-2xl font-semibold leading-tight tracking-tight text-foreground md:text-3xl">
						{title}
					</h1>
					{#if enrichment?.title && enrichment.title !== item.title}
						<p class="mt-1 text-sm text-muted">Indexed as “{item.title}”</p>
					{/if}
				</div>

				<div class="flex flex-wrap items-center gap-2 text-sm text-muted">
					{#if enrichment?.year || item.year}
						<span>{enrichment?.year || item.year}</span>
					{/if}
					{#if mediaSubtitle(item)}
						<span aria-hidden="true">·</span><span>{mediaSubtitle(item)}</span>
					{/if}
					{#if library}
						<span aria-hidden="true">·</span><span>{library.name}</span>
					{/if}
					{#if item.size > 0}
						<span aria-hidden="true">·</span><span>{formatBytes(item.size)}</span>
					{/if}
				</div>

				{#if enrichment?.genres?.length}
					<div class="flex flex-wrap gap-1.5">
						{#each enrichment.genres as genre (genre)}
							<Badge>{genre}</Badge>
						{/each}
					</div>
				{/if}

				{#if enrichment?.synopsis}
					<p class="max-w-3xl text-sm leading-relaxed text-muted">{enrichment.synopsis}</p>
				{/if}

				{#if progress && ratio > 0 && !progress.completed}
					<div class="max-w-md space-y-1.5">
						<ProgressBar {ratio} class="bg-surface-active" />
						<p class="text-xs text-muted">
							{formatTime(progress.position_sec)} watched
							{#if progress.updated_at}· {formatRelative(progress.updated_at)}{/if}
						</p>
					</div>
				{/if}

				<div class="flex flex-wrap items-center gap-3 pt-1">
					{#if playable}
						<LinkButton href={`/player/${item.id}`} size="lg">
							<Play class="size-4" />
							{resumeAt > 0 ? `Resume from ${formatTime(resumeAt)}` : 'Play'}
						</LinkButton>
					{:else}
						<Button size="lg" disabled title={plan?.reason ?? 'Not playable in the browser'}>
							<Play class="size-4" /> Play
						</Button>
					{/if}
					{#if progress && (progress.position_sec > 0 || progress.completed)}
						<Button variant="ghost" size="sm" loading={resetting} onclick={() => void resetProgress()}>
							<RotateCcw class="size-3.5" /> Start over
						</Button>
					{/if}
					{#if session.isAdmin}
						<div class="flex items-center gap-2 md:ml-auto">
							<Button variant="secondary" size="sm" loading={enriching} onclick={() => void fetchMetadata()}>
								<Sparkles class="size-3.5" /> {enrichment ? 'Refetch metadata' : 'Fetch metadata'}
							</Button>
							{#if enrichment}
								<Button variant="danger" size="sm" onclick={() => (confirmRemove = true)}>
									<Trash2 class="size-3.5" /> Remove
								</Button>
							{/if}
						</div>
					{/if}
				</div>

				{#if !playable}
					<div class="max-w-2xl rounded-card border border-warning/25 bg-warning/5 px-4 py-3 text-sm">
						<p class="font-medium text-warning">The browser cannot play this file directly.</p>
						<p class="mt-1 text-muted">
							{plan?.reason ?? 'No compatible playback plan is available.'}
							{#if session.isAdmin}
								A transcode provider is not installed; the desktop/CLI client plays it as-is.
							{:else}
								Ask an administrator, or use the desktop/CLI client.
							{/if}
						</p>
					</div>
				{/if}

				{#if enrichment}
					<p class="text-xs text-muted">
						Metadata from <span class="text-foreground">{enrichment.provider}</span>
						{#if enrichment.fetched_at}· {formatDate(enrichment.fetched_at)}{/if}
					</p>
				{/if}
				<p class="break-all font-mono text-[11px] text-muted/70">{item.file_path}</p>
			</div>
		</div>
	</article>

	<Modal bind:open={confirmRemove} title="Remove metadata overlay?" description="Identity, progress and files are untouched — only the fetched artwork and description go away.">
		{#snippet footer()}
			<Button variant="ghost" onclick={() => (confirmRemove = false)}>Cancel</Button>
			<Button variant="danger" loading={removing} onclick={() => void removeMetadata()}>
				Remove overlay
			</Button>
		{/snippet}
		<p class="text-sm text-muted">
			Provider: <span class="text-foreground">{enrichment?.provider}</span>. You can fetch it again later.
		</p>
	</Modal>
{/if}
