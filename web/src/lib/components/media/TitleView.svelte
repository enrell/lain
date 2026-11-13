<script lang="ts">
	import Ellipsis from '@lucide/svelte/icons/ellipsis';
	import EllipsisVertical from '@lucide/svelte/icons/ellipsis-vertical';
	import Circle from '@lucide/svelte/icons/circle';
	import Download from '@lucide/svelte/icons/download';
	import { downloadUrl } from '$lib/download/files';
	import RotateCcw from '@lucide/svelte/icons/rotate-ccw';
	import FavoriteButton from '$lib/components/social/FavoriteButton.svelte';
	import RatingPill from '$lib/components/social/RatingPill.svelte';
	import ProgressBar from '$lib/components/media/ProgressBar.svelte';
	import CircleCheck from '@lucide/svelte/icons/circle-check';
	import ChevronLeft from '@lucide/svelte/icons/chevron-left';
	import BookOpen from '@lucide/svelte/icons/book-open';
	import ArrowLeft from '@lucide/svelte/icons/arrow-left';
	import { onMount, tick } from 'svelte';
	import Check from '@lucide/svelte/icons/check';
	import ListChecks from '@lucide/svelte/icons/list-checks';
	import Play from '@lucide/svelte/icons/play';
	import Trash2 from '@lucide/svelte/icons/trash-2';
	import X from '@lucide/svelte/icons/x';
	import type { CatalogItem, Library, Progress } from '$lib/api/types';
	import { episodeLabel, type SeriesGroup } from '$lib/utilities/grouping';
	import { enrichmentCache } from '$lib/stores/media-cache.svelte';
	import { wideStill } from '$lib/stores/frames';
	import { api } from '$lib/api';
	import { session } from '$lib/auth/session.svelte';
	import { toasts } from '$lib/stores/toasts.svelte';
	import { errorMessage } from '$lib/utilities/errors';
	import Button from '$lib/components/primitives/Button.svelte';
	import LinkButton from '$lib/components/primitives/LinkButton.svelte';
	import Modal from '$lib/components/primitives/Modal.svelte';
	import Poster from '$lib/components/media/Poster.svelte';
	import { RAIL } from '$lib/components/media/grid';
	import DownloadFileButton from '$lib/components/media/DownloadFileButton.svelte';
	import ExternalPlayers from '$lib/components/player/ExternalPlayers.svelte';
	import { formatLength, formatTime, timeLeft } from '$lib/utilities/format';
	import { playTarget } from '$lib/utilities/progress';
	import Menu, { type MenuEntry } from '$lib/components/primitives/Menu.svelte';
	import Settings2 from '@lucide/svelte/icons/settings-2';
	import { t } from '$lib/i18n';
	import { loadPreferredPlayer, playbackHref, playExternalClick, type PreferredPlayer } from '$lib/player/external-player';
	import { isReadable, readingLabel } from '$lib/reader/kinds';
	import { usePageKeys } from '$lib/stores/page-keys.svelte';
	import { gridKeys } from '$lib/utilities/grid-nav';
	let preferredPlayer = $state<PreferredPlayer>('browser');
	let origin = $state('');
	// How long each episode is, asked once for the whole title (D-276).
	let durations = $state<Map<string, number>>(new Map());
	onMount(() => {
		origin = window.location.origin;
		if (session.user) preferredPlayer = loadPreferredPlayer(session.user.id);
		if (!isReadable(group.items[0]?.kind)) void api.playback.durations(group.items.map((i) => i.id)).then((d) => (durations = d));
	});

	/**
	 * The title page body: a cinematic backdrop with the title block, a
	 * poster rail (resume action, admin actions, information) beside the
	 * episode grid. Single-file titles (movies, specials) never reach
	 * here — the item route renders those directly.
	 *
	 * Every card plays its episode, which is why there is no per-episode
	 * page: the title owns the grid, the watch page owns the player.
	 */
	let {
		group,
		progressMap = null,
		libraries = [],
		manage = [],
		about = null,
		returned = false,
		onProgressChanged = undefined
	}: {
		group: SeriesGroup;
		progressMap?: Map<string, Progress> | null;
		libraries?: Library[];
		/** What an admin can do with the title, from the page that owns it. */
		manage?: MenuEntry[];
		/** What the files say of themselves (a comic's ComicInfo.xml), when no overlay does. */
		about?: { summary?: string; genres?: string[] } | null;
		/** The page was come back to (Back): it returns to where it was, not to the current part. */
		returned?: boolean;
		/** Called after marks changed, so the page reads progress again. */
		onProgressChanged?: () => void;
	} = $props();

	// Comics and manga open in the reader; external players never apply (D-085).
	const reading = $derived(isReadable(group.items[0]?.kind));
	const player = $derived<PreferredPlayer>(reading ? 'browser' : preferredPlayer);
	function playHref(id: string): string { return playbackHref(origin, id, player, group.items[0]?.kind); }
	function label(item: CatalogItem): string { return reading ? readingLabel(item) : episodeLabel(item); }
	// A comic split into volumes (Vol 1, Vol 2, no issue numbers) is counted
	// in volumes, as its library card does; the page said "3 issues" and
	// "Seasons 3" under a card that said "3 volumes".
	const byVolume = $derived(reading && group.items.every((i) => i.season > 0 && i.episode <= 0));
	const unitPlural = $derived(
		!reading ? 'episodes' : group.items[0]?.kind !== 'manga' ? (byVolume ? 'volumes' : 'issues') : group.items.some((i) => i.episode > 0) ? 'volumes & chapters' : 'volumes'
	);

	const seasons = $derived(
		[...new Set(group.items.map((i) => i.season).filter((s) => s > 0))].sort((a, b) => a - b)
	);
	let season = $state<number | 'all'>('all');

	const visible = $derived(
		season === 'all' ? group.items : group.items.filter((i) => i.season === season)
	);

	const first = $derived(group.items[0]);
	const artItem = $derived(group.items.find((i) => i.episode > 0) ?? first);
	const enrichments = $derived(group.items.map((i) => enrichmentCache.get(i.id)));
	const synopsis = $derived(
		enrichments.map((e) => e?.synopsis).find((s) => s && s.length > 0) ?? about?.summary ?? ''
	);
	const genres = $derived(
		(enrichments.map((e) => e?.genres ?? []).find((g) => g.length > 0) ?? about?.genres ?? []).slice(0, 3)
	);
	const backdrop = $derived(enrichments.map((e) => e?.cover).find((c) => c) ?? '');
	const posterEnrichment = $derived(enrichmentCache.get(artItem.id) ?? null);
	// The provider's title wins over the parsed one, exactly like the
	// single-title page: a scan reads "Frieren", the overlay knows
	// "Frieren: Beyond Journey's End".
	const displayTitle = $derived(
		enrichments.map((e) => e?.title).find((t) => t && t.length > 0) ?? group.title
	);

	const countLabel = $derived(
		t(!reading ? 'title.episodes' : group.items[0]?.kind === 'manga' || byVolume ? 'title.volumes' : 'title.issues', { count: group.count })
	);
	const totalLength = $derived(group.items.reduce((sum, i) => sum + (durations.get(i.id) ?? 0), 0));
	const meta = $derived([group.year > 0 ? String(group.year) : '', countLabel].filter(Boolean).join('  ·  '));

	const libraryNames = $derived(
		libraries
			.filter((lib) => group.libraryIds.includes(lib.id))
			.map((lib) => lib.name)
			.filter((name, index, all) => all.indexOf(name) === index)
	);

	const homeLibrary = $derived(libraries.find((lib) => group.libraryIds.includes(lib.id)) ?? null);
	let synopsisOpen = $state(false);

	const watched = $derived(group.items.filter((i) => progressMap?.get(i.id)?.completed).length);

	// Episode selection (D-073, admin only): a mode that turns the cards
	// into toggles instead of play links, with bulk delete routed through
	// DELETE /api/items/{id}. The server keeps the catalog row as
	// `missing`, so ids that were deleted this session render like any
	// other missing episode until the next reload confirms it.
	let selecting = $state(false);
	let selected = $state<Set<string>>(new Set());
	let deletedIds = $state<Set<string>>(new Set());
	let confirmDelete = $state(false);
	let deleting = $state(false);

	function isGone(item: CatalogItem): boolean {
		return item.missing || deletedIds.has(item.id);
	}

	function toggleSelect(id: string): void {
		const next = new Set(selected);
		if (next.has(id)) next.delete(id);
		else next.add(id);
		selected = next;
	}

	function selectAll(): void {
		selected = new Set(visible.map((i) => i.id));
	}

	function selectWatched(): void {
		selected = new Set(
			visible.filter((i) => progressMap?.get(i.id)?.completed).map((i) => i.id)
		);
	}

	function exitSelecting(): void {
		selecting = false;
		selected = new Set();
	}

	usePageKeys(() => [
		...(group.items.length > 1 ? [{ keys: ['↑', '↓', '←', '→'], label: t(reading ? 'pageKeys.partsMove' : 'pageKeys.episodesMove') }] : []),
		...(selecting ? [{ keys: ['Esc'], label: t('pageKeys.selectDone') }] : [])
	]);

	// One click for the whole title: select every episode that still has
	// a file — across all seasons, not just the visible filter — and open
	// the same confirmation the manual selection uses.
	function deleteAllFiles(): void {
		selected = new Set(group.items.filter((i) => !isGone(i)).map((i) => i.id));
		confirmDelete = true;
	}

	async function deleteSelected(): Promise<void> {
		if (deleting || selected.size === 0) return;
		deleting = true;
		const ids = [...selected];
		const gone = new Set(deletedIds);
		const failed: { id: string; err: unknown }[] = [];
		for (const id of ids) {
			try {
				await api.items.remove(id);
				gone.add(id);
			} catch (err) {
				failed.push({ id, err });
			}
		}
		deletedIds = gone;
		confirmDelete = false;
		deleting = false;
		if (failed.length === 0) {
			toasts.success(ids.length === 1 ? 'Episode deleted.' : `${ids.length} episodes deleted.`);
			exitSelecting();
		} else {
			// Keep the failures selected so it is obvious which ones stayed.
			selected = new Set(failed.map((f) => f.id));
			toasts.error(errorMessage(failed[0].err, `${failed.length} of ${ids.length} episodes could not be deleted.`));
		}
	}

	// Resume at the first started-but-unfinished episode; otherwise play
	// the show from the top like a fresh watch. Missing episodes can never
	// be the target — the plan refuses them at play time anyway.
	// Reading progress stores the 1-based page: page 1 is "just opened".
	// A book left on its first page is being read, as Home and the library already say.
	const minResume = $derived(reading ? 1 : 5);
	const available = $derived(group.items.filter((i) => !isGone(i)));
	const resumeTarget = $derived(playTarget(available, progressMap, minResume));
	const resumeProgress = $derived(
		resumeTarget ? (progressMap?.get(resumeTarget.id) ?? null) : null
	);
	const resuming = $derived(
		!!resumeProgress && !resumeProgress.completed && resumeProgress.position_sec >= minResume
	);
	// "Resume from 4:12", never "Resume · S01E01": the smoke test and the
	// item page both key on that phrase, and a bare "Resume" would also
	// match the cards' "Resume at 42%" text.
	const resumeLabel = $derived(
		reading
			? resuming
				? // Which book it continues, on a title of several: "page 3" alone did not say.
					group.items.length > 1
					? t('title.continueFromPageIn', { page: Math.floor(resumeProgress!.position_sec), part: label(resumeTarget!) })
					: t('title.continueFromPage', { page: Math.floor(resumeProgress!.position_sec) })
				: resumeTarget && group.items.length > 1
					? t('title.readPart', { part: label(resumeTarget) })
					: t('title.read')
			: resuming
				? // The phrase the smoke test keys on, then which episode it is.
					`Resume from ${formatTime(resumeProgress!.position_sec)} · ${label(resumeTarget!)}`
				: resumeTarget && resumeTarget.id !== available[0]?.id
					? // Which episode Play goes on with, once it is not the first.
						t('title.playPart', { part: label(resumeTarget) })
					: t('title.play')
	);
	const watchedLine = $derived(
		resuming && resumeProgress!.duration_sec > 0
			? reading
				? // A book begun is where you are, not "Up next" (the app reads "Vol 1: Page 3 of 8").
					[
						t('title.readingAt', { part: label(resumeTarget!), page: Math.floor(resumeProgress!.position_sec), pages: Math.floor(resumeProgress!.duration_sec) }),
						watched > 0 ? t('title.readOf', { done: watched, count: group.count }) : ''
					]
						.filter(Boolean)
						.join(' · ')
				: [timeLeft(resumeProgress!.position_sec, resumeProgress!.duration_sec), watched > 0 ? t('title.watchedOf', { done: watched, count: group.count }) : '']
						.filter(Boolean)
						.join(' · ')
			: available.length === 0
				? t('title.allMissing')
				: watched > 0
					? t(reading ? 'title.readOf' : 'title.watchedOf', { done: watched, count: group.count })
					: t('title.notStarted')
	);

	// The title's own management joins what the page passed in.
	const manageEntries = $derived<MenuEntry[]>(
		session.isAdmin
			? [
					...manage,
					{ label: t('title.selectEpisodes'), icon: ListChecks, onSelect: () => (selecting = true), separated: manage.length > 0 },
					{ label: t('title.deleteTitle'), icon: Trash2, danger: true, disabled: group.items.every(isGone), onSelect: deleteAllFiles }
				]
			: []
	);


	function cardLine(item: CatalogItem, progress: Progress | null): string {
		const length = formatLength(durations.get(item.id));
		if (progress && !progress.completed && progress.duration_sec > 0 && progress.position_sec > 0) {
			return reading
				? t('browse.pageOf', { page: Math.floor(progress.position_sec), pages: Math.floor(progress.duration_sec) })
				: timeLeft(progress.position_sec, progress.duration_sec);
		}
		if (progress?.completed) return reading ? t('title.read') : length ? t('title.watchedLength', { time: length }) : t('title.watched');
		return length || t(reading ? 'title.unread' : 'title.notWatched');
	}

	// One part's own actions, as the app's ⋮ on each row: its mark, its start
	// and its file, without selecting it or marking the whole title.
	async function markOne(item: CatalogItem, done: boolean): Promise<void> {
		const dur = durations.get(item.id) ?? progressMap?.get(item.id)?.duration_sec ?? 0;
		try {
			await api.progress.put(item.id, { position_sec: done ? dur : 0, duration_sec: dur, completed: done });
			toasts.success(t('title.marked', { part: label(item) }));
			onProgressChanged?.();
		} catch (err) {
			toasts.error(errorMessage(err, t('title.markFailed')));
		}
	}
	function downloadOne(item: CatalogItem): void {
		const a = document.createElement('a');
		a.href = downloadUrl(item.id, session.token);
		a.download = '';
		a.rel = 'noopener';
		document.body.appendChild(a);
		a.click();
		a.remove();
	}
	function partEntries(item: CatalogItem, progress: Progress | null): MenuEntry[] {
		const done = !!progress?.completed;
		const begun = !done && !!progress && progress.position_sec >= minResume;
		return [
			...(begun ? [{ label: t('title.startOver'), icon: RotateCcw, onSelect: () => void markOne(item, false) }] : []),
			{
				label: t(done ? (reading ? 'title.markUnread' : 'title.markUnwatched') : reading ? 'title.markRead' : 'title.markWatched'),
				icon: done ? Circle : CircleCheck,
				onSelect: () => void markOne(item, !done)
			},
			{ label: t('download.one'), icon: Download, onSelect: () => downloadOne(item) }
		];
	}

	// Marking the whole title watched (or not): asked first, since every part changes at once.
	let confirmMark = $state(false);
	let marking = $state(false);
	const allDone = $derived(group.count > 0 && watched === group.count);

	// One "More": the one under way back to its start, the marks of the whole title (asked first),
	// then, for administrators below a line, the title's management.
	const moreEntries = $derived<MenuEntry[]>([
		...(resuming && resumeTarget ? [{ label: t('title.startOver'), icon: RotateCcw, onSelect: () => void startOver() }] : []),
		{
			label: allDone ? (reading ? 'Mark all unread' : 'Mark all unwatched') : reading ? 'Mark all read' : 'Mark all watched',
			icon: CircleCheck,
			onSelect: () => (confirmMark = true)
		},
		...manageEntries.map((e, i) => (i === 0 ? { ...e, separated: true } : e))
	]);
	async function markAll(): Promise<void> {
		if (marking) return;
		marking = true;
		const done = !allDone;
		try {
			for (const it of group.items) {
				const dur = durations.get(it.id) ?? progressMap?.get(it.id)?.duration_sec ?? 0;
				await api.progress.put(it.id, { position_sec: done ? dur : 0, duration_sec: dur, completed: done });
			}
			toasts.success(done ? (reading ? 'All marked as read.' : 'All marked as watched.') : 'Marks removed.');
			onProgressChanged?.();
		} catch (err) {
			toasts.error(errorMessage(err, 'Could not change the marks.'));
		} finally {
			marking = false;
			confirmMark = false;
		}
	}

	let restarting = $state(false);
	async function startOver(): Promise<void> {
		if (!resumeTarget || restarting) return;
		restarting = true;
		const was = resumeProgress;
		const id = resumeTarget.id;
		try {
			await api.progress.put(id, { position_sec: 0, duration_sec: resumeProgress?.duration_sec ?? 0, completed: false });
			toasts.undoable('Back to the start.', () => {
				if (!was) return;
				void api.progress
					.put(id, { position_sec: was.position_sec, duration_sec: was.duration_sec, completed: was.completed })
					.then(() => onProgressChanged?.());
			});
			onProgressChanged?.();
		} catch (err) {
			toasts.error(errorMessage(err, 'Could not reset progress.'));
		} finally {
			restarting = false;
		}
	}

	/*
	 * On arrival the part the main button opens is brought into view, when
	 * the title is under way and it is below the fold: a viewer on Episode 8
	 * landed on Episodes 1 to 4, and on a phone on none at all. Only as far
	 * as needed, so as much of the hero stays; never on Back, which returns
	 * to where the page was, and never moving the focus.
	 */
	let arrived = false;
	$effect(() => {
		const target = resumeTarget;
		if (arrived || returned || !target || !progressMap || !(resuming || watched > 0)) return;
		arrived = true;
		void tick().then(() => {
			const card = document.querySelector<HTMLElement>(`[data-episode-grid] [data-part="${CSS.escape(target.id)}"]`);
			if (!card) return;
			// Clear of the phone's bottom bar.
			const floor = window.innerHeight - (window.matchMedia('(min-width: 768px)').matches ? 24 : 96);
			const box = card.getBoundingClientRect();
			// A card already half in view is found: nudging the page for its caption
			// only slid the top of the hero under the bar.
			if (box.top + box.height / 2 <= floor) return;
			const below = box.bottom - floor;
			if (below <= 0) return;
			const calm = window.matchMedia('(prefers-reduced-motion: reduce)').matches;
			window.scrollBy({ top: below, behavior: calm ? 'auto' : 'smooth' });
		});
	});

	function stillUrl(id: string): string {
		return api.thumbnail.url(id, session.token, { width: 480 });
	}

	function cardProgress(id: string): number {
		const p = progressMap?.get(id);
		if (!p || p.completed || p.duration_sec <= 0) return 0;
		return Math.min(100, Math.round((p.position_sec / p.duration_sec) * 100));
	}
</script>

<!-- Closer under the hero on a phone, where every row above the episodes pushes them down. -->
<article class="space-y-4 md:space-y-8">
	<!-- Full-bleed title hero in the home hero's language: no box, no
	     border; the art runs under the navigation and fades into the page. -->
	<header class="title-hero relative -mx-5 -mt-5 overflow-hidden sm:-mx-8 md:mx-0 md:-mt-28">
		{#if backdrop}
			<img src={backdrop} alt="" aria-hidden="true" class="hero-monolith-art absolute inset-y-0 right-0 h-full w-full object-cover object-center max-md:scale-150 max-md:opacity-55 max-md:blur-3xl" referrerpolicy="no-referrer" />
		{:else}
			<img src={stillUrl(artItem.id)} alt="" aria-hidden="true" class="hero-monolith-art absolute inset-y-0 right-0 h-full w-full scale-110 object-cover object-top opacity-80 blur-md" />
		{/if}
		<div class="hero-monolith-shade absolute inset-0" aria-hidden="true"></div>
		<!-- On a phone Back is the top bar's (AppShell): drawn here, it scrolled under that bar. -->
		<!-- One template for every kind of title: the cover, then what it is and what it can do.
		     Both start on one line: aligned at the bottom, the cover floated mid-hero beside a
		     long synopsis and the copy floated beside a cover taller than it. On a phone the
		     cover sits beside the name, not above it: stacked, it put every episode below
		     the fold. -->
		<div class="relative grid w-full grid-cols-[5.5rem_minmax(0,1fr)] items-end gap-x-4 gap-y-5 pb-6 pt-6 md:flex md:min-h-[26rem] md:flex-row md:items-start md:gap-10 md:pb-10 md:pt-32">
			<div class="aspect-[2/3] w-full shrink-0 overflow-hidden rounded-xl shadow-[0_24px_60px_rgba(0,0,0,0.55)] ring-1 ring-white/10 md:w-52">
				<Poster item={artItem} enrichment={posterEnrichment} priority class="size-full" />
			</div>
			<div class="hero-monolith-copy hero-monolith-split min-w-0 max-w-6xl flex-1">
				<!-- The name block: beside the cover on a phone (the parent is `contents` there). -->
				<div class="min-w-0 @container">
				{#if homeLibrary}
					<a href={`/library/${homeLibrary.id}`} class="mb-3 inline-flex items-center gap-1 text-sm text-foreground/70 hover:text-foreground max-md:hidden">
						<ChevronLeft class="size-4" aria-hidden="true" /> {homeLibrary.name}
					</a>
				{/if}
				<!-- The facts a title is chosen by, in type that reads: tiny spaced capitals were the hardest text on the page. -->
				<p class="text-sm font-medium text-foreground/80">{meta}</p>
				<h1 class="title-hero-name mt-4 text-foreground">{displayTitle}</h1>
				{#if genres.length > 0}
					<ul class="mt-4 flex flex-wrap gap-x-2 gap-y-1 text-sm text-foreground/70" aria-label="Genres">
						{#each genres as genre, i (genre)}
							<li>{#if i > 0}<span class="mr-2 text-muted" aria-hidden="true">·</span>{/if}{genre}</li>
						{/each}
					</ul>
				{/if}
				</div>
				<div class="max-md:col-span-2">
				<!-- The main action and what else the title can do, where the eye
				     already is; the rail below keeps the cover and the facts. -->
				<!-- On a phone: the main action and the heart on one line, then the score,
				     Download and More (as an icon) on the next. Spelled out, More fell onto a
				     third line of its own. -->
				<div class="title-actions flex flex-wrap items-center gap-x-2 gap-y-3 md:mt-6">
					{#if resumeTarget}
						<LinkButton href={playHref(resumeTarget.id)} size="rowMain" class="max-md:w-[calc(100%-3.25rem)]" data-title-play aria-keyshortcuts="p" onclick={(e) => playExternalClick(e, resumeTarget.id, player)}>
							{#if reading}<BookOpen class="size-4" aria-hidden="true" />{:else}<Play class="size-4 fill-current" aria-hidden="true" />{/if} {resumeLabel}
						</LinkButton>
					{:else}
						<Button size="rowMain" disabled class="max-md:w-[calc(100%-3.25rem)]" title="The files are no longer on disk">
							<Play class="size-4 fill-current" aria-hidden="true" /> {reading ? t('title.read') : t('title.play')}
						</Button>
					{/if}
					<!-- The heart keeps the main action's row, on a phone too. -->
					<FavoriteButton />
					<RatingPill />
					{#if resumeTarget && !reading}<span class="max-md:hidden"><ExternalPlayers itemId={resumeTarget.id} variant="tonal" size="row" /></span>{/if}
					<!-- D, as on a film's page; while selecting, D saves the selection instead. -->
					<DownloadFileButton items={group.items} variant="tonal" size="row" label={t('download.all')} hotkey={!selecting} />
					<Menu label={t('title.more')} hint={t('title.moreHint')} entries={moreEntries} variant="tonal" size="row" compact>
						{#snippet icon()}<Ellipsis class="size-4" aria-hidden="true" />{/snippet}
					</Menu>
				</div>
				<!-- The part under way, as the film page shows it: a bar under the button that resumes it. -->
				{#if resuming && resumeProgress!.duration_sec > 0}
					<ProgressBar ratio={resumeProgress!.position_sec / resumeProgress!.duration_sec} class="mt-4 max-w-sm bg-surface-active" />
				{/if}
				<p class="mt-3 text-sm text-muted" aria-live="polite">{watchedLine}</p>
				{#if synopsis}
					<!-- A cut synopsis opens on a tap: an ellipsis with no way past it is a dead end. -->
					<button type="button" class="mt-4 block max-w-2xl text-left" aria-expanded={synopsisOpen} onclick={() => (synopsisOpen = !synopsisOpen)}>
						<!-- Clamped, it is not a block: `block` overrode the clamp's box and three lines showed on a phone. -->
						<span class={['text-sm leading-6 text-foreground/70 sm:text-base sm:leading-7', synopsisOpen ? 'block' : 'line-clamp-2 md:line-clamp-3'].join(' ')}>{synopsis}</span>
					</button>
				{/if}
				</div>
			</div>
		</div>
	</header>

	<div>
		<div class="min-w-0">
			<!-- Chips narrow a long list; when every chip would be one card of the list under it (three volumes), they only say it twice. -->
			{#if seasons.length > 1 && seasons.length < group.items.length}
				<div class="mb-4 flex flex-wrap gap-2" role="group" aria-label="Seasons">
					<button
						type="button"
						onclick={() => (season = 'all')}
						aria-pressed={season === 'all'}
						class={[
							'rounded-full px-3.5 py-1.5 text-xs font-semibold transition-colors',
							season === 'all' ? 'bg-accent text-accent-fg' : 'bg-surface text-muted hover:text-foreground'
						].join(' ')}
					>
						All
					</button>
					{#each seasons as s (s)}
						<button
							type="button"
							onclick={() => (season = s)}
							aria-pressed={season === s}
							class={[
								'rounded-full px-3.5 py-1.5 text-xs font-semibold transition-colors',
								season === s ? 'bg-accent text-accent-fg' : 'bg-surface text-muted hover:text-foreground'
							].join(' ')}
						>
							{reading ? `Vol ${s}` : `Season ${s}`}
						</button>
					{/each}
				</div>
			{/if}

			<div class="mb-4 flex flex-wrap items-center justify-between gap-3">
				<h2 class="section-title capitalize">
					{reading ? unitPlural : 'Episodes'}
					{#if selecting}
						<span class="ml-2 align-middle font-mono text-[11px] font-normal uppercase tracking-[0.14em] text-muted">
							{selected.size} selected
						</span>
					{/if}
				</h2>
				{#if session.isAdmin}
					<div class="flex flex-wrap items-center gap-2" role="group" aria-label="Episode selection">
						{#if selecting}
							<button
								type="button"
								onclick={selectAll}
								class="rounded-full bg-surface px-3.5 py-1.5 text-xs font-semibold text-muted transition-colors hover:text-foreground"
							>
								All
							</button>
							<button
								type="button"
								onclick={selectWatched}
								class="rounded-full bg-surface px-3.5 py-1.5 text-xs font-semibold text-muted transition-colors hover:text-foreground"
							>
								Watched
							</button>
							<button
								type="button"
								onclick={() => (selected = new Set())}
								class="rounded-full bg-surface px-3.5 py-1.5 text-xs font-semibold text-muted transition-colors hover:text-foreground"
							>
								None
							</button>
							<DownloadFileButton items={group.items.filter((i) => selected.has(i.id))} hotkey />
							<Button variant="danger" size="sm" disabled={selected.size === 0} onclick={() => (confirmDelete = true)}>
								<Trash2 class="size-3.5" aria-hidden="true" /> Delete
							</Button>
							<Button variant="ghost" size="sm" onclick={exitSelecting}>
								<X class="size-3.5" aria-hidden="true" /> Done
							</Button>
						{/if}
					</div>
				{/if}
			</div>
			<!-- More, smaller cards on a wide screen: three big ones put every name below the fold. -->
			<!-- Volumes are the cover cards of the "More in" rail under them, at its
			     width from sm up (a fluid grid drew them 182 px wide over 192 px rail
			     cards); two a row on a phone, as every poster grid there. Episodes
			     as wide cards, rows on a phone. Arrows move between the parts as in a
			     library grid; up from the first row returns to Play. -->
			<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
			<ul class={reading ? `grid grid-cols-2 items-start gap-x-3.5 gap-y-6 sm:grid-cols-[repeat(auto-fill,var(--card-w))] md:gap-x-4 ${RAIL.portrait}` : 'episode-rows grid grid-cols-1 gap-3 sm:grid-cols-2 sm:gap-x-4 sm:gap-y-6 lg:grid-cols-3 xl:grid-cols-4 2xl:grid-cols-5'} data-episode-grid onkeydown={(e) => gridKeys(e, () => document.querySelector<HTMLElement>('a[data-title-play]'))}>
				{#each visible as item (item.id)}
					{@const progress = progressMap?.get(item.id) ?? null}
					{@const ratio = cardProgress(item.id)}
					{@const gone = isGone(item)}
					{@const current = item.id === resumeTarget?.id && (resuming || watched > 0)}
					<li class="relative" data-part={item.id}>
						{#if selecting}
							<!-- Selection mode turns the card into a toggle so the
							     click never reaches a play link. -->
							<button
								type="button"
								onclick={() => toggleSelect(item.id)}
								aria-pressed={selected.has(item.id)}
								class={['group relative block w-full rounded-card text-left', gone ? 'opacity-60' : ''].join(' ')}
								aria-label={`${selected.has(item.id) ? 'Deselect' : 'Select'} ${label(item)}`}
							>
								{@render episodeCard(item, progress, ratio, gone, false, selected.has(item.id), false, true)}
								<span
									class={[
										'absolute left-2 top-2 flex size-6 items-center justify-center rounded-full border backdrop-blur transition-colors',
										selected.has(item.id)
											? 'border-accent bg-accent text-accent-fg'
											: 'border-white/40 bg-black/55 text-transparent'
									].join(' ')}
								>
									<Check class="size-3.5" aria-hidden="true" />
								</span>
							</button>
						{:else if gone}
							<!-- A missing file is not a dead link pretending to play:
							     the card stays for context (progress, titles) but does
							     not navigate. -->
							<div
								class="block opacity-60"
								aria-label={`${label(item)}, missing from disk`}
							>
								{@render episodeCard(item, progress, ratio, gone)}
							</div>
						{:else}
							<a
								href={playHref(item.id)}
								onclick={(e) => playExternalClick(e, item.id, player)}
								class="group block rounded-card transition-transform duration-300 ease-out hover:-translate-y-1"
								aria-label={`${reading ? 'Read' : 'Play'} ${label(item)}${item.title !== group.title ? `, ${item.title}` : ''}`}
							>
								{@render episodeCard(item, progress, ratio, gone, true, false, current)}
							</a>
							<!-- Beside the link, not in it: a button inside a link is not a button. -->
							<span class="absolute bottom-1.5 end-1.5">
								<Menu label={t('title.partActions', { part: label(item) })} entries={partEntries(item, progress)} variant="ghost" size="icon">
									{#snippet icon()}<EllipsisVertical class="size-4" aria-hidden="true" />{/snippet}
								</Menu>
							</span>
						{/if}
					</li>
				{/each}
			</ul>
		</div>
		<!-- What is known of the title, as one quiet strip under its episodes. -->
		<section class="mt-12" aria-label="Details">
			<h2 class="font-mono text-[10px] uppercase tracking-[0.18em] text-muted">Details</h2>
			<dl class="mt-3 grid grid-cols-2 gap-x-8 text-sm sm:grid-cols-3 lg:grid-cols-6">
					{#if group.year > 0}
						<div class="border-t border-line/60 py-3">
							<dt class="text-xs uppercase tracking-wider text-muted">Year</dt>
							<dd class="mt-1 font-semibold text-foreground">{group.year}</dd>
						</div>
					{/if}
					<!-- A reading file's season is its volume: the count below says it. -->
					{#if seasons.length > 0 && !reading}
						<div class="border-t border-line/60 py-3">
							<dt class="text-xs uppercase tracking-wider text-muted">Seasons</dt>
							<dd class="mt-1 font-semibold text-foreground">{seasons.length}</dd>
						</div>
					{/if}
					<div class="border-t border-line/60 py-3">
						<dt class="text-xs uppercase tracking-wider text-muted">{reading ? (group.items[0]?.kind === 'manga' || byVolume ? 'Volumes' : 'Issues') : 'Episodes'}</dt>
						<dd class="mt-1 font-semibold text-foreground">{group.count}</dd>
					</div>
					{#if totalLength > 0}
						<div class="border-t border-line/60 py-3">
							<dt class="text-xs uppercase tracking-wider text-muted">{t('title.length')}</dt>
							<dd class="mt-1 font-semibold text-foreground">{formatLength(totalLength)}</dd>
						</div>
					{/if}
					{#if libraryNames.length > 0}
						<div class="border-t border-line/60 py-3">
							<dt class="text-xs uppercase tracking-wider text-muted">Library</dt>
							<dd class="mt-1 truncate font-semibold text-foreground">{libraryNames.join(' · ')}</dd>
						</div>
					{/if}
					<div class="border-t border-line/60 py-3">
						<dt class="text-xs uppercase tracking-wider text-muted">{reading ? 'Read' : 'Watched'}</dt>
						<dd class="mt-1 font-semibold text-foreground">{watched} of {group.count}</dd>
					</div>
				</dl>
		</section>
	</div>
</article>

{#snippet episodeCard(item: CatalogItem, progress: Progress | null, ratio: number, gone: boolean, menu = false, chosen = false, current = false, choosing = false)}
	<!-- A card as every other card in the app is drawn: the picture in a rounded
	     frame with a faint ring, the words under it. A bordered box around both
	     made the episodes and volumes a second kind of card. -->
	<div
		class={[
			'episode-thumb relative overflow-hidden rounded-xl bg-surface shadow-[0_18px_45px_rgba(0,0,0,0.16)] transition duration-300',
			reading ? 'aspect-[2/3]' : 'aspect-video',
			// The part the main button opens wears the accent, so it is found at a glance.
			chosen ? 'ring-2 ring-accent' : current ? 'ring-2 ring-accent/70' : 'ring-1 ring-white/5 group-hover:ring-white/15'
		].join(' ')}
	>
		<img
			src={wideStill(item, 480)}
			alt=""
			aria-hidden="true"
			loading="lazy"
			decoding="async"
			class="size-full object-cover {reading ? 'object-top' : ''} transition-transform duration-500 ease-out group-hover:scale-[1.035]"
		/>
		<!-- No play badge while choosing: the card then toggles, it does not play. -->
		{#if !gone && !choosing}
			<span
				class="absolute right-2 top-2 flex size-8 items-center justify-center rounded-full bg-black/65 text-white opacity-0 backdrop-blur transition-opacity duration-200 group-hover:opacity-100"
			>
				<Play class="size-3.5 translate-x-px" aria-hidden="true" />
			</span>
		{/if}
		{#if progress?.completed}
			<span class="absolute left-2 top-2 flex size-6 items-center justify-center rounded-full bg-black/65 text-white backdrop-blur" aria-hidden="true">
				<Check class="size-3.5" />
			</span>
		{/if}
		{#if ratio > 0}
			<!-- The bar every card wears: inset and rounded, not a strip on the edge. -->
			<div class="absolute inset-x-0 bottom-0 p-1.5">
				<ProgressBar ratio={ratio / 100} label={t(reading ? 'title.readProgress' : 'title.watchProgress')} class="bg-black/50" />
			</div>
		{/if}
	</div>
	<!-- Room at the end for the card's ⋮, which sits over this corner. -->
	<div class="episode-text min-w-0 pt-3" style:padding-inline-end={menu ? '2.75rem' : undefined}>
		<p class="truncate text-sm font-semibold leading-snug tracking-[-0.015em] text-foreground">{label(item)}</p>
		{#if item.title !== group.title}
			<p class="mt-0.5 truncate text-sm text-foreground/80">{item.title}</p>
		{/if}
		<p class={['mt-1 truncate text-xs', gone ? 'text-danger' : 'text-muted'].join(' ')}>
			{gone ? 'Missing from disk' : cardLine(item, progress)}
		</p>
	</div>
{/snippet}

<svelte:window
	onkeydown={(e) => {
		if (e.key === 'Escape' && selecting && !confirmDelete) exitSelecting();
	}}
/>

<Modal
	bind:open={confirmDelete}
	title={`Delete ${selected.size} ${selected.size === 1 ? 'episode' : 'episodes'}?`}
	description="The files are removed from disk for every user. The catalog entries stay as missing, so progress and metadata are preserved and an episode restores itself if its file returns."
>
	{#snippet footer()}
		<Button variant="ghost" onclick={() => (confirmDelete = false)}>Cancel</Button>
		<Button variant="danger" loading={deleting} onclick={() => void deleteSelected()}>
			Delete {selected.size === 1 ? 'file' : `${selected.size} files`}
		</Button>
	{/snippet}
	<p class="text-sm text-muted">This cannot be undone — the media bytes are gone.</p>
</Modal>

<Modal
	bind:open={confirmMark}
	title={allDone ? 'Remove every mark?' : reading ? 'Mark all as read?' : 'Mark all as watched?'}
	description={`${countLabel} of ${displayTitle} change at once. Progress inside each one is replaced.`}
>
	{#snippet footer()}
		<Button variant="ghost" onclick={() => (confirmMark = false)}>Cancel</Button>
		<Button loading={marking} onclick={() => void markAll()}>{allDone ? 'Remove marks' : 'Mark all'}</Button>
	{/snippet}
</Modal>
