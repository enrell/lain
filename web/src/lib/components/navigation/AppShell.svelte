<script lang="ts">
	import { onDestroy, onMount } from 'svelte';
	import type { Snippet } from 'svelte';
	import { page } from '$app/state';
	import SignalMark from '../primitives/SignalMark.svelte';
	import UserMenu from './UserMenu.svelte';
	import MobileNav from './MobileNav.svelte';
	import { serverStatus } from '$lib/stores/server-status.svelte';

	let { children }: { children: Snippet } = $props();

	// Monolith navigation: quiet centered text links. Per-type entries
	// (anime/shows/movies) live in /library and the home rails, not here.
	const links = [
		{ href: '/', label: 'Home', exact: true },
		{ href: '/library', label: 'Library', exact: false },
		{ href: '/search', label: 'Search', exact: false },
		{ href: '/settings', label: 'Settings', exact: false }
	];

	function active(href: string, exact: boolean): boolean {
		const path = page.url.pathname;
		return exact ? path === href : path === href || path.startsWith(href + '/');
	}

	onMount(() => {
		serverStatus.start();
	});

	onDestroy(() => serverStatus.stop());
</script>

<div class="min-h-dvh">
	<header class="fixed inset-x-0 top-0 z-40 hidden border-b border-white/5 bg-background/55 backdrop-blur-xl md:block">
		<div class="mx-auto grid h-20 w-full max-w-[1800px] grid-cols-[1fr_auto_1fr] items-center px-6 lg:px-10">
			<a href="/" class="flex shrink-0 items-center gap-2.5 justify-self-start" aria-label="Lain home">
				<SignalMark class="size-5 text-accent" />
				<span class="text-base font-bold tracking-[0.24em] text-foreground">lain</span>
			</a>

			<nav class="flex items-center gap-8" aria-label="Primary">
				{#each links as item (item.label)}
					<a
						href={item.href}
						aria-current={active(item.href, item.exact) ? 'page' : undefined}
						class={[
							'text-[13px] tracking-wide transition-colors',
							active(item.href, item.exact) ? 'text-foreground' : 'text-muted hover:text-foreground'
						].join(' ')}
					>
						{item.label}
					</a>
				{/each}
			</nav>

			<div class="flex shrink-0 items-center gap-4 justify-self-end">
				<a
					href="/settings"
					class="flex items-center gap-2 text-[10px] font-semibold uppercase tracking-[0.14em] text-muted transition-colors hover:text-foreground"
					title={`Server ${serverStatus.state}`}
					aria-label={`Server ${serverStatus.state}`}
				>
					<span
						class={[
							'size-1.5 rounded-full',
							serverStatus.state === 'online'
								? 'bg-accent shadow-[0_0_12px_var(--color-accent)]'
								: serverStatus.state === 'checking'
									? 'animate-pulse bg-muted'
									: 'bg-danger'
						].join(' ')}
						aria-hidden="true"
					></span>
					<span class="hidden xl:inline">{serverStatus.state}</span>
				</a>
				<UserMenu side="bottom" compact />
			</div>
		</div>
	</header>

	<header class="sticky top-0 z-30 flex items-center justify-between border-b border-line bg-background/88 px-4 py-3 backdrop-blur-xl md:hidden">
		<a href="/" class="flex items-center gap-2" aria-label="Lain home">
			<SignalMark class="size-5 text-accent" />
			<span class="font-semibold tracking-[0.2em] text-foreground">lain</span>
		</a>
		<div class="w-40"><UserMenu /></div>
	</header>

	<main>
		<div class={page.url.pathname === '/' ? 'w-full pb-24 md:pb-0' : 'mx-auto w-full max-w-[1600px] px-4 pb-28 pt-6 md:px-8 md:pb-14 md:pt-28'}>
			{@render children()}
		</div>
	</main>

	<MobileNav />
</div>
