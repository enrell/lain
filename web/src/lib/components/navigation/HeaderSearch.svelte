<script lang="ts">
	/*
	 * The header's search field (md and up). It finds titles only: settings
	 * and actions are Quick find's (Ctrl+K), which has no button of its own.
	 * Away from /search it offers the top titles as you type and Enter opens
	 * every result; on /search it is that page's field, and the results
	 * below follow what is typed. The field never moves or disappears, so
	 * a search can be refined from wherever it was started.
	 */
	import { untrack } from 'svelte';
	import CornerDownLeft from '@lucide/svelte/icons/corner-down-left';
	import History from '@lucide/svelte/icons/rotate-ccw-clock';
	import Search from '@lucide/svelte/icons/search';
	import X from '@lucide/svelte/icons/x';
	import { afterNavigate, goto } from '$app/navigation';
	import { page } from '$app/state';
	import { api } from '$lib/api';
	import { t } from '$lib/i18n';
	import Poster from '$lib/components/media/Poster.svelte';
	import { isReadable } from '$lib/reader/kinds';
	import { enrichmentCache, ensureEnrichments } from '$lib/stores/media-cache.svelte';
	import { recentSearches } from '$lib/stores/recent-searches.svelte';
	import { artItem } from '$lib/utilities/browse';
	import { mediaSubtitle } from '$lib/utilities/format';
	import { groupItems, type SeriesGroup } from '$lib/utilities/grouping';
	import { isTypingTarget } from '$lib/utilities/guards';
	import { searchHref } from '$lib/utilities/search';

	/** Titles offered under the field; the rest are one Enter away. */
	const TOP = 6;

	interface Row {
		id: string;
		kind: 'all' | 'title' | 'recent';
		label: string;
		href: string;
		group?: SeriesGroup;
	}

	let text = $state('');
	let focused = $state(false);
	let input = $state<HTMLInputElement>();
	let groups = $state<SeriesGroup[]>([]);
	let loading = $state(false);
	let failed = $state(false);
	let active = $state(-1);
	// Where focus was before the field took it: Escape on an empty field goes back there.
	let back: HTMLElement | null = null;

	const onSearchPage = $derived(page.url.pathname === '/search');
	const term = $derived(text.trim());
	// Holding a query or focused, the field looks in use. Below lg it opens out over the links
	// while focused (AppShell hides them meanwhile).
	const inUse = $derived(focused || text !== '');

	// Row ids end up in aria-activedescendant, a single id: nothing with spaces (a term, a title) goes in them.
	const rows = $derived.by((): Row[] => {
		if (onSearchPage) return [];
		if (term === '') {
			return recentSearches.items.map((r, i) => ({ id: `recent-${i}`, kind: 'recent', label: r, href: searchHref(r) }));
		}
		return [
			{ id: 'all', kind: 'all', label: t('search.field.all', { query: term }), href: searchHref(term) },
			...groups.map((g): Row => {
				const id = artItem(g).id;
				return { id: `title-${id}`, kind: 'title', label: g.title, href: `/item/${id}`, group: g };
			})
		];
	});
	const open = $derived(focused && !onSearchPage && (term !== '' || rows.length > 0));

	// Enter with a query means every result, so that row starts chosen;
	// with none, nothing is until an arrow picks a recent search.
	$effect(() => {
		void rows;
		active = untrack(() => term) === '' ? -1 : 0;
	});

	// Titles come from the server; a stale answer never overwrites a newer one.
	let seq = 0;
	$effect(() => {
		const q = term;
		const mine = ++seq;
		if (onSearchPage || q.length < 2) {
			groups = [];
			loading = false;
			failed = false;
			return;
		}
		loading = true;
		const timer = setTimeout(async () => {
			try {
				// A page as large as the results page's: a long series can fill a small one alone.
				const res = await api.search.query({ q, limit: 60 });
				if (mine !== seq) return;
				const top = groupItems(res.items).slice(0, TOP);
				groups = top;
				failed = false;
				void ensureEnrichments(top.map((g) => artItem(g).id));
			} catch {
				if (mine === seq) {
					groups = [];
					failed = true;
				}
			} finally {
				if (mine === seq) loading = false;
			}
		}, 140);
		return () => clearTimeout(timer);
	});

	/*
	 * On /search the field holds that page's query. A recent search chosen
	 * on the page, its own clear, or Back change the URL, and the field
	 * follows; the echo of this field's own typing must not overwrite what
	 * has been typed since it was sent.
	 */
	let written: string | null = null;
	$effect(() => {
		if (!onSearchPage) {
			written = null;
			return;
		}
		const q = page.url.searchParams.get('q') ?? '';
		if (q === written) return;
		written = q;
		if (untrack(() => text.trim()) !== q) text = q;
	});

	// Leaving the results for a title, or anywhere else, puts the field back to rest.
	afterNavigate(() => {
		if (page.url.pathname !== '/search' && document.activeElement !== input) text = '';
	});

	let urlTimer: ReturnType<typeof setTimeout> | undefined;
	/** On /search: put the typed query in the URL, which the page's results follow. */
	function follow(now: boolean): void {
		clearTimeout(urlTimer);
		const go = (): void => {
			const href = searchHref(text);
			written = text.trim();
			if (`${page.url.pathname}${page.url.search}` !== href) {
				void goto(href, { replaceState: true, keepFocus: true, noScroll: true });
			}
		};
		if (now) go();
		else urlTimer = setTimeout(go, 250);
	}

	function choose(row: Row): void {
		if (row.kind === 'title') {
			recentSearches.remember(term);
			text = '';
			input?.blur();
			void goto(row.href);
			return;
		}
		// Every result, or a recent search again: the field keeps the query and the focus.
		const q = row.kind === 'recent' ? row.label : term;
		recentSearches.remember(q);
		text = q;
		written = q;
		void goto(row.href, { keepFocus: true });
	}

	function clear(): void {
		text = '';
		if (onSearchPage) follow(true);
		input?.focus();
	}

	function onKey(e: KeyboardEvent): void {
		const down = e.key === 'ArrowDown' || (e.ctrlKey && e.key === 'n');
		const up = e.key === 'ArrowUp' || (e.ctrlKey && e.key === 'p');
		if (e.key === 'Escape') {
			e.preventDefault();
			if (text !== '') clear();
			else {
				input?.blur();
				back?.focus?.();
			}
		} else if (onSearchPage) {
			if (e.key === 'Enter') {
				e.preventDefault();
				recentSearches.remember(text);
				follow(true);
			} else if (down) {
				// The results are the page: the arrow walks into them.
				const first = document.querySelector<HTMLElement>('main a[href^="/item/"]');
				if (first) {
					e.preventDefault();
					first.focus();
				}
			}
		} else if (down || up) {
			e.preventDefault();
			if (rows.length === 0) return;
			if (active < 0) active = down ? 0 : rows.length - 1;
			else active = (active + (down ? 1 : -1) + rows.length) % rows.length;
		} else if (e.key === 'Enter') {
			e.preventDefault();
			const row = rows[active];
			if (row) choose(row);
			else void goto('/search', { keepFocus: true });
		}
	}

	/*
	 * `/` focuses the field from anywhere, at once, so the letters typed
	 * right after it land here. A page with a field of its own for `/`
	 * (Library's filter, Acquire, the server's downloads) still wins: its
	 * handler registers after the shell's, judges the key by its original
	 * target, and moves focus on to its own field.
	 */
	function onWindowKey(e: KeyboardEvent): void {
		if (e.key !== '/' || e.ctrlKey || e.metaKey || e.altKey) return;
		if (isTypingTarget(e.target) || document.querySelector('[role="dialog"]')) return;
		// A handler that ran first already put the cursor somewhere.
		if (isTypingTarget(document.activeElement)) return;
		// Hidden on a phone's layout: the key is the page's there.
		if (!input || input.getClientRects().length === 0) return;
		// Firefox would open its own find bar on the key.
		e.preventDefault();
		input.focus();
		input.select();
	}

	// The line the result cards show: how many parts and the year for a series, a lone file's own line.
	function detail(g: SeriesGroup): string {
		const item = g.items[0];
		if (g.count === 1) return mediaSubtitle(item);
		const parts = [t(!isReadable(item.kind) ? 'title.episodes' : item.kind === 'manga' ? 'title.volumes' : 'title.issues', { count: g.count })];
		if (g.year > 0) parts.push(String(g.year));
		return parts.join(' · ');
	}
</script>

<svelte:window onkeydown={onWindowKey} />

<!-- The slot grows with the header's free space; the field inside opens over the links when it needs more. -->
<div class="@container relative h-9 min-w-[4.5rem] max-w-72 flex-1">
	<div class={['absolute end-0 top-0 h-9 w-full', focused ? 'max-lg:min-w-72' : ''].join(' ')}>
		<Search class="pointer-events-none absolute start-3 top-1/2 size-3.5 -translate-y-1/2 text-muted" aria-hidden="true" />
		<input
			bind:this={input}
			bind:value={text}
			id="header-search"
			type="search"
			role="combobox"
			aria-label={t('search.field.label')}
			aria-keyshortcuts="/"
			aria-autocomplete="list"
			aria-expanded={open}
			aria-controls="header-search-list"
			aria-activedescendant={open && rows[active] ? `header-search-${rows[active].id}` : undefined}
			placeholder={t('search.field.placeholder')}
			autocomplete="off"
			autocapitalize="none"
			spellcheck={false}
			oninput={() => {
				if (onSearchPage) follow(false);
			}}
			onkeydown={onKey}
			onfocus={(e) => {
				focused = true;
				back = e.relatedTarget instanceof HTMLElement ? e.relatedTarget : null;
			}}
			onblur={() => (focused = false)}
			class={[
				'h-9 w-full rounded-full border ps-8 pe-9 text-xs text-foreground outline-none transition-colors placeholder:text-muted [&::-webkit-search-cancel-button]:hidden',
				inUse ? 'border-accent/50 bg-surface' : 'cursor-pointer border-line/70 bg-surface/40 hover:bg-surface-hover',
				// Too narrow for its words, the closed field is a glass and a key cap.
				focused ? '' : '@max-[9rem]:placeholder:text-transparent'
			].join(' ')}
		/>
		{#if text !== ''}
			<button
				type="button"
				tabindex="-1"
				class="absolute inset-y-0 end-0 flex w-9 items-center justify-center rounded-full text-muted hover:text-foreground"
				aria-label={t('search.clear')}
				onmousedown={(e) => e.preventDefault()}
				onclick={clear}
			>
				<X class="size-3.5" aria-hidden="true" />
			</button>
		{:else if !focused}
			<kbd class="pointer-events-none absolute end-2.5 top-1/2 -translate-y-1/2 text-muted">/</kbd>
		{/if}
	</div>

	{#if open}
		<div
			class="absolute end-0 top-[calc(100%+0.5rem)] z-50 w-[min(26rem,calc(100vw-2.5rem))] overflow-hidden rounded-xl border border-hairline bg-background/95 shadow-[0_30px_80px_rgba(0,0,0,0.6)] backdrop-blur-xl"
		>
			<!-- Rows keep the field's focus: a press on one must not blur it before the click lands. -->
			<!-- svelte-ignore a11y_click_events_have_key_events -->
			<ul id="header-search-list" role="listbox" aria-label={t('search.field.results')} class="max-h-[60vh] overflow-y-auto py-1.5">
				{#each rows as row, i (row.id)}
					{#if row.kind === 'recent' && i === 0}
						<li role="presentation" class="px-4 pb-1 pt-2 font-mono text-[9px] uppercase tracking-[0.26em] text-muted">{t('search.recent')}</li>
					{:else if row.kind === 'title' && rows[i - 1]?.kind !== 'title'}
						<li role="presentation" class="px-4 pb-1 pt-2.5 font-mono text-[9px] uppercase tracking-[0.26em] text-muted">{t('search.field.results')}</li>
					{/if}
					<li
						id="header-search-{row.id}"
						role="option"
						aria-selected={i === active}
						class={[
							'mx-1.5 flex cursor-default items-center gap-3 rounded-md px-2.5 text-sm',
							row.kind === 'title' ? 'py-1.5' : 'py-2',
							i === active ? 'bg-surface-active text-foreground' : 'text-foreground/85'
						].join(' ')}
						onpointermove={() => (active = i)}
						onmousedown={(e) => e.preventDefault()}
						onclick={() => choose(row)}
					>
						{#if row.kind === 'title' && row.group}
							{@const art = artItem(row.group)}
							<Poster item={art} enrichment={enrichmentCache.get(art.id) ?? null} class="h-12 w-8 shrink-0 rounded-sm" />
							<span class="min-w-0 flex-1">
								<span class="block truncate font-medium">{row.label}</span>
								<span class="block truncate text-xs text-muted">{detail(row.group)}</span>
							</span>
						{:else}
							{#if row.kind === 'recent'}
								<History class="size-3.5 shrink-0 text-muted" aria-hidden="true" />
							{:else}
								<Search class="size-3.5 shrink-0 text-accent" aria-hidden="true" />
							{/if}
							<span class="min-w-0 flex-1 truncate">{row.label}</span>
						{/if}
						{#if i === active}
							<CornerDownLeft class="size-3.5 shrink-0 text-muted" aria-hidden="true" />
						{/if}
					</li>
				{/each}
			</ul>
			{#if term !== ''}
				{#if groups.length === 0 && term.length >= 2}
					<p class="px-4 pb-3 pt-1 text-xs text-muted" aria-live="polite">
						{loading ? t('search.field.searching') : failed ? t('search.field.failed') : t('search.field.none', { query: term })}
					</p>
				{/if}
			{/if}
			<div class="flex items-center gap-4 border-t border-hairline px-4 py-2 font-mono text-[10px] text-muted">
				<span><kbd>↑↓</kbd> {t('search.field.move')}</span>
				<span><kbd>↵</kbd> {t('search.field.open')}</span>
				<span><kbd>Esc</kbd> {t(text !== '' ? 'search.field.clear' : 'search.field.close')}</span>
				<!-- Quick find has no button: the place people look for "find" names its key. -->
				<span class="ms-auto"><kbd>Ctrl K</kbd> {t('search.field.quickFind')}</span>
			</div>
		</div>
	{/if}
</div>
