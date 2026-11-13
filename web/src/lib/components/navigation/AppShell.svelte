<script lang="ts">
	import { i18n, t } from '$lib/i18n';
	import type { Snippet } from 'svelte';
	import { page } from '$app/state';
	import { afterNavigate, goto } from '$app/navigation';
	import ArrowLeft from '@lucide/svelte/icons/arrow-left';
	import SignalMark from '../primitives/SignalMark.svelte';
	import UserMenu from './UserMenu.svelte';
	import MobileNav from './MobileNav.svelte';
	import CommandPalette from './CommandPalette.svelte';
	import HeaderSearch from './HeaderSearch.svelte';
	import KeyboardHelp from './KeyboardHelp.svelte';
	import RailKeys from './RailKeys.svelte';
	import { scan } from '$lib/stores/scan.svelte';
	import { socialBadge } from '$lib/stores/social.svelte';
	import { session } from '$lib/auth/session.svelte';

	let { children }: { children: Snippet } = $props();

	// Monolith navigation: quiet centered text links. Per-type entries
	// (anime/shows/movies) live in /library and the home rails, not here.
	// Search is not a link: the header holds the field itself (HeaderSearch).
	const links = $derived([
		{ href: '/', label: t('nav.home'), exact: true },
		{ href: '/library', label: t('nav.library'), exact: false },
		{ href: '/favorites', label: t('nav.favorites'), exact: false },
		{ href: '/list', label: t('nav.list'), exact: false },
		{ href: '/social', label: t('nav.social'), exact: false },
		// Acquisition is admin-only (docs/slices/acquisition.md, A-9).
		...(session.isAdmin ? [{ href: '/acquire', label: t('nav.acquire'), exact: false }] : []),
		{ href: '/settings', label: t('nav.settings'), exact: false }
	]);

	function active(href: string, exact: boolean): boolean {
		const path = page.url.pathname;
		return exact ? path === href : path === href || path.startsWith(href + '/');
	}

	// The shell only mounts for a signed-in session: poll the social badge meanwhile.
	$effect(() => {
		socialBadge.start();
		return () => socialBadge.stop();
	});

	// The shell owns scan polling: the header stays live on every page,
	// and a scan started elsewhere (the watcher, another tab) shows on load.
	$effect(() => {
		void scan.watch();
		return () => scan.stop();
	});

	/*
	 * A title page on a phone goes back from the bar that stays on screen.
	 * The arrow floated on the hero scrolled under this bar with the page, so
	 * a title opened onto its current volume showed two pixels of it.
	 * Opened from outside (a shared link), there is nothing of the app to go
	 * back to, so it leads to the library instead of leaving the app.
	 */
	const backable = $derived(page.route.id === '/item/[id]');
	let cameFromApp = false;
	afterNavigate((nav) => {
		if (nav.from !== null && nav.type !== 'enter') cameFromApp = true;
	});
	function goBack(): void {
		if (cameFromApp) history.back();
		else void goto('/library');
	}

	const scanLabel = $derived(
		scan.fraction === null
			? t('nav.scanning')
			: t('nav.scanningPercent', { percent: i18n.formatNumber(scan.fraction, { style: 'percent' }) })
	);

</script>

<div class="min-h-dvh">
	<!-- First focusable element: the header repeats on every page. -->
	<a
		href="#main"
		class="sr-only focus:not-sr-only focus:absolute focus:start-4 focus:top-4 focus:z-50 focus:rounded-md focus:border focus:border-line focus:bg-surface focus:px-4 focus:py-2 focus:text-sm focus:text-foreground"
	>
		{t('nav.skip')}
	</a>

	<header class="group/header fixed inset-x-0 top-0 z-40 hidden border-b border-white/5 bg-background/55 backdrop-blur-xl md:block">
		<div class="mx-auto grid h-20 w-full max-w-[1800px] grid-cols-[1fr_auto_1fr] items-center px-5 sm:px-8 lg:px-10">
			<a href="/" class="flex shrink-0 items-center gap-2.5 justify-self-start" aria-label={t('common.home')}>
				<SignalMark class="size-5 text-accent" />
				<span class="text-base font-bold tracking-[0.24em] text-foreground">lain</span>
			</a>

			<!-- Tighter below xl, so the search field keeps room for its words. Below lg
			     the focused field opens over the links, and they step aside meanwhile. -->
			<nav
				class="flex items-center gap-5 xl:gap-8 max-lg:group-has-[#header-search:focus]/header:invisible"
				aria-label={t('nav.primary')}
			>
				{#each links as item (item.label)}
					<a
						href={item.href}
						aria-current={active(item.href, item.exact) ? 'page' : undefined}
						class={[
							'relative inline-flex h-20 items-center text-[13px] tracking-wide transition-colors',
							// The page you are on: brighter, heavier, and underlined in the accent.
							active(item.href, item.exact)
								? "font-semibold text-foreground after:absolute after:inset-x-0 after:bottom-0 after:h-0.5 after:rounded-full after:bg-accent after:content-['']"
								: 'text-muted hover:text-foreground'
						].join(' ')}
					>
						{item.label}
						{#if item.href === '/social' && socialBadge.unread > 0}
							<span class="ms-1.5 rounded-full bg-accent px-1.5 font-mono text-[10px] text-background" aria-label={t('nav.unread', { count: socialBadge.unread })}>{socialBadge.unread}</span>
						{/if}
					</a>
				{/each}
			</nav>

			<!-- Stretched over its column, so the search field can take what the links leave free. -->
			<div class="flex items-center justify-end gap-4 ps-4 xl:ps-8">
				<!-- Operational state earns the header only while it is happening. -->
				{#if scan.running}
					<a href="/settings/libraries#scan" class="flex shrink-0 items-center gap-2 font-mono text-[10px] uppercase tracking-[0.18em] text-accent">
						<span class="size-1.5 animate-pulse rounded-full bg-accent" aria-hidden="true"></span><span class="tabular-nums" title={scan.line}>{scanLabel}</span>
					</a>
				{/if}
				<HeaderSearch />
				<UserMenu side="bottom" compact />
			</div>
		</div>
	</header>

	<header
		class={['sticky top-0 z-30 flex items-center justify-between border-b border-line bg-background/92 pr-3 backdrop-blur-xl md:hidden', backable ? 'pl-1' : 'pl-5'].join(' ')}
		style="padding-top: env(safe-area-inset-top);"
	>
		<div class="flex items-center gap-1">
			{#if backable}
				<button type="button" class="flex size-11 items-center justify-center rounded-full text-foreground active:bg-surface-hover" aria-label={t('nav.back')} onclick={goBack}>
					<ArrowLeft class="size-5" aria-hidden="true" />
				</button>
			{/if}
			<a href="/" class="flex h-12 items-center gap-2" aria-label={t('common.home')}>
				<SignalMark class="size-[18px] text-accent" />
				<span class="text-[15px] font-semibold tracking-[0.2em] text-foreground">lain</span>
			</a>
		</div>
		<div class="flex items-center gap-1">
			{#if scan.running}
				<span class="flex items-center gap-1.5 pr-1 font-mono text-[10px] uppercase tracking-[0.14em] text-muted" aria-live="polite">
					<span class="size-1.5 animate-pulse rounded-full bg-accent"></span>{scanLabel}
				</span>
			{/if}
			<UserMenu side="bottom" compact />
		</div>
	</header>

	<!-- Quick find: Ctrl+K only, named in the field's hints, the `?` list and the settings rail. -->
	<CommandPalette />
	<KeyboardHelp />
	<RailKeys />

	<main id="main" tabindex="-1" class="overflow-x-clip">
		<div class={page.url.pathname === '/' ? 'w-full pb-24 md:pb-0' : 'mx-auto w-full max-w-[1800px] px-5 sm:px-8 lg:px-10 pb-[calc(6rem+env(safe-area-inset-bottom))] pt-5 md:pb-14 md:pt-28'}>
			{@render children()}
		</div>
	</main>

	<MobileNav />
</div>
