<script lang="ts">
	import House from '@lucide/svelte/icons/house';
	import Library from '@lucide/svelte/icons/library';
	import Search from '@lucide/svelte/icons/search';
	import Settings from '@lucide/svelte/icons/settings';
	import { page } from '$app/state';

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

<nav
	class="fixed inset-x-0 bottom-0 z-40 border-t border-line bg-background/95 backdrop-blur md:hidden"
	aria-label="Primary"
	style="padding-bottom: env(safe-area-inset-bottom);"
>
	<div class="grid grid-cols-4">
		{#each items as item (item.href)}
			{@const isActive = active(item.href, item.exact)}
			{@const Icon = item.icon}
			<a
				href={item.href}
				aria-current={isActive ? 'page' : undefined}
				class={[
					'flex flex-col items-center gap-1 py-2.5 text-[11px] font-medium transition-colors',
					isActive ? 'text-accent' : 'text-muted hover:text-foreground'
				].join(' ')}
			>
				<Icon class="size-5" />
				{item.label}
			</a>
		{/each}
	</div>
</nav>
