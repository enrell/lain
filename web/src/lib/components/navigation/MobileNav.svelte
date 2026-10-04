<script lang="ts">
	import { t } from '$lib/i18n';
	import House from '@lucide/svelte/icons/house';
	import Library from '@lucide/svelte/icons/library';
	import ListChecks from '@lucide/svelte/icons/list-checks';
	import Search from '@lucide/svelte/icons/search';
	import Settings from '@lucide/svelte/icons/settings';
	import Users from '@lucide/svelte/icons/users';
	import { socialBadge } from '$lib/stores/social.svelte';
	import { page } from '$app/state';

	const items = $derived([
		{ href: '/', label: t('nav.home'), icon: House, exact: true },
		{ href: '/library', label: t('nav.library'), icon: Library, exact: false },
		{ href: '/list', label: t('nav.list'), icon: ListChecks, exact: false },
		{ href: '/social', label: t('nav.social'), icon: Users, exact: false },
		{ href: '/search', label: t('nav.search'), icon: Search, exact: false },
		{ href: '/settings', label: t('nav.settings'), icon: Settings, exact: false }
	]);

	function active(href: string, exact: boolean): boolean {
		const path = page.url.pathname;
		return exact ? path === href : path === href || path.startsWith(href + '/');
	}
</script>

<nav
	class="fixed inset-x-0 bottom-0 z-40 border-t border-line bg-background/95 backdrop-blur md:hidden"
	aria-label={t('nav.primary')}
	style="padding-bottom: env(safe-area-inset-bottom);"
>
	<div class="grid grid-cols-6">
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
				<span class="relative">
					<Icon class="size-5" />
					{#if item.href === '/social' && socialBadge.unread > 0}
						<span class="absolute -end-1 -top-1 size-2 rounded-full bg-accent" aria-label={t('nav.unread', { count: socialBadge.unread })}></span>
					{/if}
				</span>
				{item.label}
			</a>
		{/each}
	</div>
</nav>
