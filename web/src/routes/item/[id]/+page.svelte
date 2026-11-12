<script lang="ts">
	import CircleCheck from '@lucide/svelte/icons/circle-check';
	import RelatedRow from '$lib/components/media/RelatedRow.svelte';
	import FavoriteButton from '$lib/components/social/FavoriteButton.svelte';
	import ChevronLeft from '@lucide/svelte/icons/chevron-left';
	import { onMount } from 'svelte';
	import { page } from '$app/state';
	import { goto } from '$app/navigation';
	import ArrowLeft from '@lucide/svelte/icons/arrow-left';
	import Clapperboard from '@lucide/svelte/icons/clapperboard';
	import Play from '@lucide/svelte/icons/play';
	import RotateCcw from '@lucide/svelte/icons/rotate-ccw';
	import Captions from '@lucide/svelte/icons/captions';
	import Sparkles from '@lucide/svelte/icons/sparkles';
	import Trash2 from '@lucide/svelte/icons/trash-2';
	import type { CatalogItem, Enrichment, Library, PlaybackPlan, Progress } from '$lib/api/types';
	import { api, ApiError } from '$lib/api';
	import { session } from '$lib/auth/session.svelte';
	import Menu, { type MenuEntry } from '$lib/components/primitives/Menu.svelte';
	import Settings2 from '@lucide/svelte/icons/settings-2';
	import ChevronRight from '@lucide/svelte/icons/chevron-right';
	import Button from '$lib/components/primitives/Button.svelte';
	import EmptyState from '$lib/components/primitives/EmptyState.svelte';
	import ErrorState from '$lib/components/primitives/ErrorState.svelte';
	import LinkButton from '$lib/components/primitives/LinkButton.svelte';
	import Modal from '$lib/components/primitives/Modal.svelte';
	import Poster from '$lib/components/media/Poster.svelte';
	import ExternalPlayers from '$lib/components/player/ExternalPlayers.svelte';
	import ProgressBar from '$lib/components/media/ProgressBar.svelte';
	import Skeleton from '$lib/components/primitives/Skeleton.svelte';
	import TitleView from '$lib/components/media/TitleView.svelte';
	import TitleSocial from '$lib/components/social/TitleSocial.svelte';
	import SubtitlePanel from '$lib/components/acquire/SubtitlePanel.svelte';
	import { t } from '$lib/i18n';
	import { isTypingTarget } from '$lib/utilities/guards';
	import {
		applyEnrichment,
		ensureEnrichments,
		ensureLibraries,
		itemCache
	} from '$lib/stores/media-cache.svelte';
	import { toasts } from '$lib/stores/toasts.svelte';
	import { errorMessage } from '$lib/utilities/errors';
	import { groupFromEpisodes, type SeriesGroup } from '$lib/utilities/grouping';
	import { formatBytes, formatDate, formatLength, formatTime } from '$lib/utilities/format';
	import { progressRatio } from '$lib/utilities/progress';
	import { loadPreferredPlayer, playbackHref, playExternalClick, type PreferredPlayer } from '$lib/player/external-player';
	import { isReadable } from '$lib/reader/kinds';
	import ComicInfoPanel from '$lib/components/reader/ComicInfoPanel.svelte';
	import DownloadFileButton from '$lib/components/media/DownloadFileButton.svelte';
	let preferredPlayer = $state<PreferredPlayer>('browser');
	let origin = $state('');
	function playHref(itemId: string): string { return playbackHref(origin, itemId, preferredPlayer, item?.kind); }

	const id = $derived(page.params.id ?? '');

	let item = $state<CatalogItem | null>(null);
	let enrichment = $state<Enrichment | null>(null);
	let progress = $state<Progress | null>(null);
	let progressMap = $state<Map<string, Progress>>(new Map());
	let plan = $state<PlaybackPlan | null>(null);
	let library = $state<Library | undefined>(undefined);
	let libraries = $state<Library[]>([]);
	// The title's other files. Null means "this is a single-file title"
	// (a movie or a lone special), which renders the plain item page.
	let series = $state<SeriesGroup | null>(null);
	let loading = $state(true);
	let error = $state<string | null>(null);
	let notFound = $state(false);

	let enriching = $state(false);
	let resetting = $state(false);
	let confirmRemove = $state(false);
	let removing = $state(false);
	let confirmDeleteFile = $state(false);
	let deletingFile = $state(false);
	let backdropFailedId = $state<string | null>(null);

	// Cover, else poster, else a wide still extracted by the server.
	const backdropSrc = $derived(
		enrichment?.cover ||
			enrichment?.poster ||
			(item ? api.thumbnail.url(item.id, session.token, { at: 30, width: 960 }) : '')
	);

	// The picture's size, as people name it (1080p), from the file's own video stream.
	const videoHeight = $derived(plan?.streams?.find((st) => st.type === 'video')?.height ?? 0);

	// One line of what it is: year, length, picture size. Each fact once; the
	// file's size and path are an admin's business (File details).
	const metaSegments = $derived(
		[
			enrichment?.year || item?.year ? String(enrichment?.year || item?.year) : '',
			formatLength(plan?.duration_sec),
			videoHeight ? `${videoHeight}p` : ''
		].filter((segment) => segment !== '')
	);

	async function load(): Promise<void> {
		loading = true;
		error = null;
		notFound = false;
		try {
			const [it, pl, enr, libs, eps, allProgress] = await Promise.all([
				api.catalog.get(id),
				// A comic or manga has no playback plan; the read never blocks the page.
				api.playback.plan(id).catch(() => null),
				api.enrich.get(id),
				ensureLibraries(),
				api.catalog.episodes(id),
				api.me.continueWatching()
			]);
			item = it;
			itemCache.set(it.id, it);
			plan = pl;
			enrichment = enr;
			if (enr) applyEnrichment(enr);
			libraries = libs;
			library = libs.find((lib) => lib.id === it.library_id);
			progressMap = new Map(allProgress.map((p) => [p.item_id, p]));
			progress = progressMap.get(it.id) ?? null;
			series = eps.items.length > 1 ? groupFromEpisodes(eps.items) : null;
			if (series) await ensureEnrichments(series.items.map((episode) => episode.id));
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
		origin = window.location.origin;
		if (session.user) preferredPlayer = loadPreferredPlayer(session.user.id);
		void load();
	});

	// Comics and manga open in the reader: no plan, no external player (D-085).
	const reading = $derived(isReadable(item?.kind));
	const playable = $derived(reading ? !item?.missing : plan?.available === true);
	const resumeAt = $derived(
		progress && !progress.completed && progress.position_sec >= (reading ? 2 : 5) ? progress.position_sec : 0
	);
	const title = $derived(enrichment?.title || item?.title || '');

	// Subtitles (admin, docs/slices/acquisition.md Phase 3): the panel
	// covers this file, or every present file of the title; t opens it.
	let subtitlesOpen = $state(false);
	const subtitleItems = $derived(
		reading || !item ? [] : (series ? series.items : [item]).filter((it) => !it.missing).map((it) => it.id)
	);
	function pageKey(e: KeyboardEvent): void {
		if (e.key !== 't' || e.ctrlKey || e.metaKey || e.altKey || isTypingTarget(e.target) || document.querySelector('[role="dialog"]')) return;
		if (!session.isAdmin || subtitleItems.length === 0) return;
		e.preventDefault();
		subtitlesOpen = true;
	}
	const ratio = $derived(progress ? progressRatio(progress) : 0);

	// What an admin can do with the title, behind one "Manage" menu. The
	// entries act on the item the route opened, so removing one overlay can
	// never strip a sibling episode's metadata.
	const manage = $derived<MenuEntry[]>(
		session.isAdmin
			? [
					{ label: t(enrichment ? 'title.refetchMetadata' : 'title.fetchMetadata'), icon: Sparkles, disabled: enriching, onSelect: () => void fetchMetadata() },
					...(subtitleItems.length ? [{ label: t('acquire.subtitles.itemAction'), icon: Captions, kbd: 't', onSelect: () => (subtitlesOpen = true) }] : []),
					...(enrichment ? [{ label: t('title.removeMetadata'), icon: Trash2, onSelect: () => (confirmRemove = true) }] : [])
				]
			: []
	);
	const manageSingle = $derived<MenuEntry[]>(
		session.isAdmin && item && !item.missing
			? [...manage, { label: t('title.deleteFile'), icon: Trash2, danger: true, separated: true, onSelect: () => (confirmDeleteFile = true) }]
			: manage
	);

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

	// File delete (admin): the media bytes go away for every user; the
	// catalog row stays as `missing`, so progress and metadata survive
	// and the item restores itself if the file comes back (D-073).
	async function deleteFile(): Promise<void> {
		if (!item || deletingFile) return;
		deletingFile = true;
		try {
			await api.items.remove(item.id);
			toasts.success('File deleted.');
			await goto('/library');
		} catch (err) {
			toasts.error(errorMessage(err, 'Could not delete the file.'));
			confirmDeleteFile = false;
		} finally {
			deletingFile = false;
		}
	}

	// Marking watched (or not) by hand: anyone may, for a title seen elsewhere.
	let marking = $state(false);
	async function markWatched(done: boolean): Promise<void> {
		if (!item || marking) return;
		marking = true;
		try {
			const dur = plan?.duration_sec ?? progress?.duration_sec ?? 0;
			progress = await api.progress.put(item.id, { position_sec: done ? dur : 0, duration_sec: dur, completed: done });
			toasts.success(done ? (reading ? 'Marked as read.' : 'Marked as watched.') : 'Mark removed.');
		} catch (err) {
			toasts.error(errorMessage(err, 'Could not change the mark.'));
		} finally {
			marking = false;
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

<svelte:head><title>{title || series?.title || 'Item'} — Lain</title></svelte:head>
<svelte:window onkeydown={pageKey} />

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
{:else if series}
	{#key series.key}
		<TitleView group={series} {progressMap} {libraries} {manage} onProgressChanged={() => void load()} />
	{/key}
	{#if reading && item}<div class="mt-8"><ComicInfoPanel {item} /></div>{/if}
{:else if item}
	<article class="space-y-6">
		<!-- The same hero as a series: the art across the top, fading into
		     the page, with the title, what it is and everything it can do. -->
		<header class="title-hero relative -mx-5 -mt-5 overflow-hidden sm:-mx-8 md:mx-0 md:-mt-28">
			{#if backdropSrc && backdropFailedId !== item.id}
				<img
					src={backdropSrc}
					alt=""
					aria-hidden="true"
					class="hero-monolith-art absolute inset-0 size-full scale-125 object-cover opacity-50 blur-3xl"
					referrerpolicy="no-referrer"
					onerror={() => (backdropFailedId = item?.id ?? null)}
				/>
			{/if}
			<div class="hero-monolith-shade absolute inset-0" aria-hidden="true"></div>
			<button type="button" class="absolute left-3 top-3 z-10 flex size-11 items-center justify-center rounded-full bg-black/45 text-white backdrop-blur md:hidden" aria-label="Back" onclick={() => history.back()}>
				<ArrowLeft class="size-5" aria-hidden="true" />
			</button>
			<div class="relative mx-auto flex w-full max-w-[1800px] flex-col gap-5 px-5 pb-6 pt-16 sm:px-8 md:min-h-[26rem] md:flex-row md:items-end md:gap-10 md:pb-10 md:pt-32 lg:px-10">
				<div class="w-24 shrink-0 md:w-52">
					<Poster {item} {enrichment} class="aspect-[2/3] rounded-xl shadow-[0_24px_60px_rgba(0,0,0,0.55)] ring-1 ring-white/10" priority />
				</div>
				<div class="hero-monolith-copy min-w-0 flex-1">
					{#if library}
						<a href={`/library/${library.id}`} class="mb-3 inline-flex items-center gap-1 text-sm text-foreground/70 hover:text-foreground max-md:hidden">
							<ChevronLeft class="size-4" aria-hidden="true" /> {library.name}
						</a>
					{/if}
					{#if metaSegments.length > 0}
						<p class="font-mono text-[10px] font-semibold uppercase tracking-[0.22em] text-accent sm:text-[11px]">{metaSegments.join('  ·  ')}</p>
					{/if}
					<h1 class="title-hero-name mt-3 text-foreground">{title}</h1>
					{#if enrichment?.genres?.length}
						<ul class="mt-4 flex flex-wrap gap-x-4 gap-y-1 font-mono text-[11px] uppercase tracking-[0.14em] text-foreground/75" aria-label="Genres">
							{#each enrichment.genres.slice(0, 4) as genre, i (genre)}
								<li>{#if i > 0}<span class="mr-4 text-muted" aria-hidden="true">/</span>{/if}{genre}</li>
							{/each}
						</ul>
					{/if}

					<!-- The primary action sits directly under the title, above the
					     optional prose, so it is on screen on arrival at 320x568. -->
					<div class="title-actions mt-6 flex flex-wrap items-center gap-x-2 gap-y-3">
						{#if playable || (!item.missing && preferredPlayer !== 'browser')}
							<LinkButton href={playHref(item.id)} size="lg" class="max-md:w-[calc(100%-3.25rem)]" onclick={(e) => playExternalClick(e, item!.id, reading ? 'browser' : preferredPlayer)}>
								<Play class="size-4 fill-current" />
								{reading ? (resumeAt > 0 ? `Continue from page ${Math.floor(resumeAt)}` : 'Read') : resumeAt > 0 ? `Resume from ${formatTime(resumeAt)}` : 'Play'}
							</LinkButton>
						{:else}
							<Button size="lg" class="max-md:w-[calc(100%-3.25rem)]" disabled title={item.missing ? 'The file is no longer on disk' : (plan?.reason ?? 'Not playable in the browser')}>
								<Play class="size-4" /> Play
							</Button>
						{/if}
						<!-- The heart keeps the main action's row, on a phone too. -->
						<FavoriteButton />
						{#if progress && (progress.position_sec > 0 || progress.completed)}
							<Button variant="ghost" size="md" loading={resetting} onclick={() => void resetProgress()}>
								<RotateCcw class="size-4" /> {t('title.startOver')}
							</Button>
						{/if}
						<Button variant="ghost" size="md" loading={marking} onclick={() => void markWatched(!progress?.completed)}>
							<CircleCheck class="size-4" /> {progress?.completed ? (reading ? 'Mark unread' : 'Mark unwatched') : reading ? 'Mark read' : 'Mark watched'}
						</Button>
						{#if !item.missing && !reading}<span class="max-md:hidden"><ExternalPlayers itemId={item.id} size="md" /></span>{/if}
						{#if !item.missing}<DownloadFileButton items={[item]} hotkey variant="ghost" size="md" label={t('download.one')} />{/if}
						<Menu label={t('title.manage')} hint={t('title.manageHint')} entries={manageSingle} variant="ghost" size="md">
							{#snippet icon()}<Settings2 class="size-4" aria-hidden="true" />{/snippet}
						</Menu>
					</div>

					{#if progress && ratio > 0 && !progress.completed}
						<div class="mt-4 max-w-sm space-y-1.5">
							<ProgressBar {ratio} class="bg-surface-active" />
							<p class="text-xs text-muted">
								{#if reading}
									Page {Math.floor(progress.position_sec)} of {Math.floor(progress.duration_sec)}
								{:else}
									{t('title.left', { time: formatLength(Math.max(60, progress.duration_sec - progress.position_sec)) })}
								{/if}
							</p>
						</div>
					{/if}

					{#if enrichment?.synopsis}
						<p class="mt-5 max-w-2xl text-sm leading-6 text-foreground/70 sm:text-base sm:leading-7">{enrichment.synopsis}</p>
					{/if}
				</div>
			</div>
		</header>

		{#if item.missing}
			<div class="max-w-2xl rounded-card border border-danger/25 bg-danger/5 px-4 py-3 text-sm">
				<p class="font-medium text-danger">The file is no longer on disk.</p>
				<p class="mt-1 text-muted">
					It stays in the catalog so progress and metadata are preserved. If the file comes
					back under {library?.name ?? 'its library'}, it becomes playable again on its own.
				</p>
			</div>
		{:else if !playable && !reading}
			<div class="max-w-2xl rounded-card border border-warning/25 bg-warning/5 px-4 py-3 text-sm">
				<p class="font-medium text-warning">The browser cannot play this file directly.</p>
				<p class="mt-1 text-muted">
					{plan?.reason ?? 'No compatible playback plan is available.'}
					{#if session.isAdmin}
						The transcoder is unavailable; the desktop/CLI client plays it as-is.
					{:else}
						Ask an administrator, or use the desktop/CLI client.
					{/if}
				</p>
			</div>
		{/if}

		{#if reading}<ComicInfoPanel {item} />{/if}

		<!-- Where the file is and where its metadata came from: an admin's
		     business, folded away from the page everyone reads. -->
		{#if session.isAdmin}
			<details class="max-w-3xl text-sm text-muted">
				<summary class="group/sum inline-flex min-h-9 cursor-pointer list-none items-center gap-1.5 rounded-md px-1 font-mono text-[10px] uppercase tracking-[0.18em] hover:text-foreground [&::-webkit-details-marker]:hidden">
					<ChevronRight class="size-3.5 transition-transform [details[open]_&]:rotate-90" aria-hidden="true" />{t('title.fileDetails')}
				</summary>
				<dl class="mt-2 space-y-1.5 border-l border-line pl-4 text-xs">
					{#if enrichment?.title && enrichment.title !== item.title}
						<div><dt class="inline text-muted">Indexed as</dt> <dd class="inline text-foreground">“{item.title}”</dd></div>
					{/if}
					{#if enrichment}
						<div><dt class="inline text-muted">Metadata from</dt> <dd class="inline text-foreground">{enrichment.provider}{#if enrichment.fetched_at} · {formatDate(enrichment.fetched_at)}{/if}</dd></div>
					{/if}
					{#if item.size > 0}<div><dt class="inline text-muted">Size</dt> <dd class="inline text-foreground">{formatBytes(item.size)}</dd></div>{/if}
					<div><dt class="sr-only">Path</dt> <dd class="break-all font-mono text-[11px]">{item.file_path}</dd></div>
				</dl>
			</details>
		{/if}
	</article>
{/if}

{#if !loading && !notFound && !error && item}
	<RelatedRow libraryId={item.library_id} exclude={series ? series.items.map((i) => i.id) : [item.id]} heading={library ? `More in ${library.name}` : 'More like this'} />
{/if}

<!-- Social slice (docs/slices/social.md): ratings, comments, sharing for the work. -->
{#if !loading && !notFound && !error && item}
	<div class="mt-12"><TitleSocial target={{ item_id: item.id }} /></div>
{/if}

{#if session.isAdmin && subtitleItems.length}
	<SubtitlePanel bind:open={subtitlesOpen} itemIds={subtitleItems} title={series?.title || title} />
{/if}

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

<Modal bind:open={confirmDeleteFile} title="Delete file?" description="The file is removed from disk for every user. The catalog entry stays as missing, so progress and metadata are preserved and the item restores itself if the file returns.">
	{#snippet footer()}
		<Button variant="ghost" onclick={() => (confirmDeleteFile = false)}>Cancel</Button>
		<Button variant="danger" loading={deletingFile} onclick={() => void deleteFile()}>
			Delete file
		</Button>
	{/snippet}
	<p class="break-all font-mono text-xs text-muted">{item?.file_path}</p>
	<p class="text-sm text-muted">This cannot be undone — the media bytes are gone.</p>
</Modal>
