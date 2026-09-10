<script lang="ts">
	import House from '@lucide/svelte/icons/house';
	import Library from '@lucide/svelte/icons/library';
	import Search from '@lucide/svelte/icons/search';
	import Settings from '@lucide/svelte/icons/settings';
	import type { Snippet } from 'svelte';
	import { page } from '$app/state';
	import SignalMark from '../primitives/SignalMark.svelte';
	import UserMenu from './UserMenu.svelte';
	import MobileNav from './MobileNav.svelte';

	let { children }: { children: Snippet } = $props();

	const items = [
		{ href: '/', label: 'Home', icon: House, exact: true },
		{ href: '/library', label: 'Library', icon: Library, exact: false },
		{ href: '/search', label: 'Search', icon: Search, exact: false },
		{ href: '/settings', label: 'Settings', icon: Settings, exact: false }
	];

	function active(href: string, exact: boolean): boolean {
		const path = page.url.pathname;
		return exact ? path === href : path === href || path.startsWith(href + '/');
	}
</script>

<div class="min-h-dvh">
	<!-- Desktop rail: quiet chrome, the artwork owns the color. -->
	<aside class="fixed inset-y-0 left-0 z-40 hidden w-56 flex-col border-r border-line bg-surface/30 md:flex">
		<a href="/" class="flex items-center gap-2.5 px-5 py-6" aria-label="Lain home">
			<SignalMark class="size-6 text-accent" />
			<span class="text-lg font-semibold tracking-[0.2em] text-foreground">lain</span>
		</a>
		<nav class="flex-1 space-y-1 px-3" aria-label="Primary">
			{#each items as item (item.href)}
				{@const isActive = active(item.href, item.exact)}
				{@const Icon = item.icon}
				<a
					href={item.href}
					aria-current={isActive ? 'page' : undefined}
					class={[
						'flex items-center gap-3 rounded-md px-3 py-2 text-sm font-medium transition-colors',
						isActive
							? 'bg-surface-active text-foreground'
							: 'text-muted hover:bg-surface-hover hover:text-foreground'
					].join(' ')}
				>
					<Icon class="size-4" />
					{item.label}
				</a>
			{/each}
		</nav>
		<div class="border-t border-line p-3">
			<UserMenu />
		</div>
	</aside>

	<!-- Mobile header. -->
	<header
		class="sticky top-0 z-30 flex items-center justify-between border-b border-line bg-background/90 px-4 py-3 backdrop-blur md:hidden"
	>
		<a href="/" class="flex items-center gap-2" aria-label="Lain home">
			<SignalMark class="size-5 text-accent" />
			<span class="font-semibold tracking-[0.2em] text-foreground">lain</span>
		</a>
		<div class="w-40">
			<UserMenu />
		</div>
	</header>

	<main class="md:pl-56">
		<div class="mx-auto w-full max-w-[1600px] px-4 pb-28 pt-5 md:px-8 md:pb-14 md:pt-8">
			{@render children()}
		</div>
	</main>

	<MobileNav />
</div>
