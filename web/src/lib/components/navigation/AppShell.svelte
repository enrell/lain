<script lang="ts">
	import { t } from '$lib/i18n';
	import type { Snippet } from 'svelte';
	import { page } from '$app/state';
	import SignalMark from '../primitives/SignalMark.svelte';
	import UserMenu from './UserMenu.svelte';
	import MobileNav from './MobileNav.svelte';
	import CommandPalette from './CommandPalette.svelte';
	import { scan } from '$lib/stores/scan.svelte';
	import { session } from '$lib/auth/session.svelte';

	let { children }: { children: Snippet } = $props();

	// Monolith navigation: quiet centered text links. Per-type entries
	// (anime/shows/movies) live in /library and the home rails, not here.
	const links = $derived([
		{ href: '/', label: t('nav.home'), exact: true },
		{ href: '/library', label: t('nav.library'), exact: false },
		{ href: '/list', label: t('nav.list'), exact: false },
		{ href: '/search', label: t('nav.search'), exact: false },
		// Acquisition is admin-only (docs/slices/acquisition.md, A-9).
		...(session.isAdmin ? [{ href: '/acquire', label: t('nav.acquire'), exact: false }] : []),
		{ href: '/settings', label: t('nav.settings'), exact: false }
	]);

	function active(href: string, exact: boolean): boolean {
		const path = page.url.pathname;
		return exact ? path === href : path === href || path.startsWith(href + '/');
	}

</script>

<div class="min-h-dvh">
	<!-- First focusable element: the header repeats on every page. -->
	<a
		href="#main"
		class="sr-only focus:not-sr-only focus:absolute focus:start-4 focus:top-4 focus:z-50 focus:rounded-md focus:border focus:border-line focus:bg-surface focus:px-4 focus:py-2 focus:text-sm focus:text-foreground"
	>
		{t('nav.skip')}
	</a>

	<header class="fixed inset-x-0 top-0 z-40 hidden border-b border-white/5 bg-background/55 backdrop-blur-xl md:block">
		<div class="mx-auto grid h-20 w-full max-w-[1800px] grid-cols-[1fr_auto_1fr] items-center px-5 sm:px-8 lg:px-10">
			<a href="/" class="flex shrink-0 items-center gap-2.5 justify-self-start" aria-label={t('common.home')}>
				<SignalMark class="size-5 text-accent" />
				<span class="text-base font-bold tracking-[0.24em] text-foreground">lain</span>
			</a>

			<nav class="flex items-center gap-8" aria-label={t('nav.primary')}>
				{#each links as item (item.label)}
					<a
						href={item.href}
						aria-current={active(item.href, item.exact) ? 'page' : undefined}
						class={[
							'inline-flex min-h-6 items-center text-[13px] tracking-wide transition-colors',
							active(item.href, item.exact) ? 'text-foreground' : 'text-muted hover:text-foreground'
						].join(' ')}
					>
						{item.label}
					</a>
				{/each}
			</nav>

			<div class="flex shrink-0 items-center gap-4 justify-self-end">
				<!-- Operational state earns the header only while it is happening. -->
				{#if scan.running}
					<a href="/settings/libraries#scan" class="flex items-center gap-2 font-mono text-[10px] uppercase tracking-[0.18em] text-accent">
						<span class="size-1.5 animate-pulse rounded-full bg-accent" aria-hidden="true"></span>{t('nav.scanning')}
					</a>
				{/if}
				<button
					type="button"
					class="hidden items-center gap-2 rounded-md px-2 py-1 font-mono text-[10px] text-muted transition-colors hover:bg-surface-hover hover:text-foreground lg:flex"
					onclick={() => window.dispatchEvent(new KeyboardEvent('keydown', { key: 'k', ctrlKey: true }))}
					aria-label={t('nav.palette')}
				>
					<kbd>Ctrl K</kbd>
				</button>
				<UserMenu side="bottom" compact />
			</div>
		</div>
	</header>

	<header class="sticky top-0 z-30 flex items-center justify-between border-b border-line bg-background/88 px-5 py-3 backdrop-blur-xl md:hidden">
		<a href="/" class="flex items-center gap-2" aria-label={t('common.home')}>
			<SignalMark class="size-5 text-accent" />
			<span class="font-semibold tracking-[0.2em] text-foreground">lain</span>
		</a>
		<div class="w-40"><UserMenu /></div>
	</header>

	<CommandPalette />

	<main id="main" tabindex="-1" class="overflow-x-clip">
		<div class={page.url.pathname === '/' ? 'w-full pb-24 md:pb-0' : 'mx-auto w-full max-w-[1800px] px-5 sm:px-8 lg:px-10 pb-28 pt-6 md:pb-14 md:pt-28'}>
			{@render children()}
		</div>
	</main>

	<MobileNav />
</div>
