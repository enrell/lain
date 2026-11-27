<script lang="ts">
	import { onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import Check from '@lucide/svelte/icons/check';
	import Info from '@lucide/svelte/icons/info';
	import { session } from '$lib/auth/session.svelte';
	import { loadPreferredPlayer, playbackHref, playExternalClick, type PreferredPlayer } from '$lib/player/external-player';
	import { playsDirectly } from '$lib/utilities/browse';
	import { enrichmentCache } from '$lib/stores/media-cache.svelte';
	import { isReadable } from '$lib/reader/kinds';
	import Play from '@lucide/svelte/icons/play';
	import type { CatalogItem, Enrichment, Progress } from '$lib/api/types';
	import { formatLength, mediaSubtitle, timeLeft } from '$lib/utilities/format';
	import { lengthOf, wantLength } from '$lib/stores/lengths.svelte';
	import { progressRatio } from '$lib/utilities/progress';
	import { t } from '$lib/i18n';
	import type { Shape } from '$lib/stores/shapes.svelte';
	import Poster from './Poster.svelte';
	import OfflineMark from './OfflineMark.svelte';
	import CardProgress from './CardProgress.svelte';

	let {
		item,
		enrichment = null,
		progress = null,
		priority = false,
		frame = undefined,
		single = false
	}: {
		item: CatalogItem;
		enrichment?: Enrichment | null;
		progress?: Progress | null;
		priority?: boolean;
		/** Fill the container's width in the frame its grid or rail shares
		 * (see `guessFrame`); a picture of the other shape is contained. */
		frame?: Shape;
		/** The card stands for a whole title (one file): it may then play
		 * the title instead of opening its page (see `playsDirectly`). */
		single?: boolean;
	} = $props();

	let player = $state<PreferredPlayer>('browser');
	let origin = $state('');
	onMount(() => {
		origin = window.location.origin;
		if (session.user) player = loadPreferredPlayer(session.user.id);
	});
	const detailsHref = $derived(`/item/${item.id}`);
	// Decided once the overlay is known (`null`: none): a film whose
	// synopsis has not arrived yet must not play on an early click.
	const direct = $derived(single && enrichmentCache.has(item.id) && playsDirectly(item, enrichment ?? enrichmentCache.get(item.id)));
	const href = $derived(direct ? playbackHref(origin, item.id, player, item.kind) : detailsHref);
	function play(e: MouseEvent): void {
		if (direct) playExternalClick(e, item.id, player);
	}
	// I on a card that plays opens the page it skips.
	function key(e: KeyboardEvent): void {
		if (!direct || e.ctrlKey || e.metaKey || e.altKey || (e.key !== 'i' && e.key !== 'I')) return;
		e.preventDefault();
		void goto(detailsHref);
	}

	const title = $derived(enrichment?.title || item.title);

	const said = $derived(
		[enrichment?.year && item.year === 0 ? String(enrichment.year) : '', mediaSubtitle(item)]
			.filter(Boolean)
			.join(' · ')
	);
	// A film or clip with nothing else to say is captioned with its length.
	const bare = $derived(said === '' && !isReadable(item.kind));
	$effect(() => {
		if (bare) wantLength(item.id);
	});
	// Under way, the line says how far, as a show's card and the hero do: what
	// is left of a film, the page of a volume. Continue reading said only
	// "Vol 1" under a bar, where Continue watching says "8 sec left".
	const under = $derived(!!progress && !progress.completed && progressRatio(progress) > 0);
	const left = $derived(under && !isReadable(item.kind) ? timeLeft(progress!.position_sec, progress!.duration_sec) : '');
	const page = $derived(under && isReadable(item.kind) ? { page: Math.max(1, Math.floor(progress!.position_sec)), pages: Math.floor(progress!.duration_sec) } : null);
	const subtitle = $derived(
		page
			? said
				? t('browse.partPage', { part: said, ...page })
				: t('browse.pageOf', page)
			: [said || (bare && !left ? formatLength(lengthOf(item.id)) : ''), left].filter(Boolean).join(' · ')
	);
	const reading = $derived(isReadable(item.kind));
	const ratio = $derived(progress && !progress.completed ? progressRatio(progress) : 0);
	// The link's aria-label replaces its inner text, so the visual 'Watching'
	// badge has to be part of the name or a screen reader never hears it.
	const accessibleName = $derived(
		[item.offline ? t('unreachable.cardLabel', { title }) : direct ? t('browse.playTitle', { title }) : title, subtitle, ratio > 0 ? (reading ? 'reading' : 'watching') : ''].filter(Boolean).join(', ')
	);
</script>

<div class="group/card relative shrink-0">
<a
	{href}
	onclick={play}
	onkeydown={key}
	aria-keyshortcuts={direct ? 'I' : undefined}
	class="group block shrink-0 rounded-card transition-transform duration-300 ease-out hover:-translate-y-1 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent"
	aria-label={accessibleName}
>
	<div
		class="relative overflow-hidden rounded-xl bg-surface shadow-[0_18px_45px_rgba(0,0,0,0.16)] ring-1 ring-white/5 transition duration-300 group-hover:ring-white/15"
	>
		<div class={frame === 'landscape' ? 'aspect-video' : 'aspect-[2/3]'}>
			<Poster {item} {enrichment} {priority} {frame} class="size-full transition-transform duration-500 ease-out group-hover:scale-[1.035]" />
		</div>
		<div
			class="pointer-events-none absolute inset-0 bg-gradient-to-t from-black/80 via-black/10 to-transparent opacity-15 transition-opacity duration-300 group-hover:opacity-100 group-focus-visible:opacity-100"
		></div>
		{#if item.offline}
			<OfflineMark compact={ratio > 0 || !!progress?.completed} class="absolute right-2 top-2" />
		{:else}
			<div
				class="pointer-events-none absolute right-2 top-2 flex size-9 items-center justify-center rounded-full bg-black/65 text-foreground opacity-0 backdrop-blur transition-opacity duration-150 group-hover:opacity-100 group-focus-visible:opacity-100"
			>
				<Play class="size-4 translate-x-px" aria-hidden="true" />
			</div>
		{/if}
		{#if ratio > 0}
			<span
				class="absolute left-2 top-2 rounded-full bg-black/65 px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wider text-foreground backdrop-blur"
			>
				{reading ? 'Reading' : 'Watching'}
			</span>
		{/if}
		{#if progress?.completed}
			<!-- Finished: a check in the corner, as the app and the episode cards mark it. -->
			<span
				class="absolute left-2 top-2 flex items-center gap-1 rounded-full bg-black/65 px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wider text-foreground backdrop-blur"
			>
				<Check class="size-3" aria-hidden="true" />{reading ? 'Read' : 'Watched'}
			</span>
		{/if}
		{#if ratio > 0}
			<CardProgress {ratio} label={t(reading ? 'title.readProgress' : 'title.watchProgress')} />
		{/if}
	</div>
	<div class="mt-3 min-w-0">
		<p class="line-clamp-2 text-sm font-semibold leading-snug tracking-[-0.015em] text-foreground">{title}</p>
		{#if subtitle}
			<p class="mt-1 truncate text-xs text-muted">{subtitle}</p>
		{/if}
	</div>
</a>
{#if direct}
	<!-- The page this card skips: a sibling link (a link inside a link is not
	     allowed), shown on hover and focus; I on the focused card opens it too. -->
	<a
		href={detailsHref}
		class="absolute left-2 top-2 flex size-8 items-center justify-center rounded-full bg-black/65 text-foreground opacity-0 backdrop-blur transition-opacity duration-150 hover:text-accent focus-visible:opacity-100 focus-visible:outline-2 focus-visible:outline-accent group-hover/card:opacity-100 group-focus-within/card:opacity-100 {ratio > 0 || progress?.completed ? 'top-9' : ''}"
		aria-label={t('browse.detailsFor', { title })}
		title={t('browse.detailsFor', { title })}
	>
		<Info class="size-4" aria-hidden="true" />
	</a>
{/if}
</div>
