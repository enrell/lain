<script lang="ts">
	import { t } from '$lib/i18n';
	import House from '@lucide/svelte/icons/house';
	import Library from '@lucide/svelte/icons/library';
	import ListChecks from '@lucide/svelte/icons/list-checks';
	import Search from '@lucide/svelte/icons/search';
	import Settings from '@lucide/svelte/icons/settings';
	import Download from '@lucide/svelte/icons/download';
	import { page } from '$app/state';
	import { session } from '$lib/auth/session.svelte';
	import { MOBILE_NAV, visibleNav } from './nav-items';

	const icons: Record<string, typeof House> = {
		'/': House,
		'/library': Library,
		'/list': ListChecks,
		'/search': Search,
		'/acquire': Download,
		'/settings': Settings
	};
	const items = $derived(
		visibleNav(MOBILE_NAV, session.isAdmin).map((e) => ({ ...e, label: t(e.label), icon: icons[e.href] ?? House }))
	);

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
	<!-- Columns follow the item count: admins see one more (Acquire). -->
	<div class="grid" style:grid-template-columns="repeat({items.length}, minmax(0, 1fr))">
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
