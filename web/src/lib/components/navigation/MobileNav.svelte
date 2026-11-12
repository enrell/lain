<script lang="ts">
	/*
	 * The phone's bottom bar: the four places used every day, and More for
	 * the rest. Seven labels in a row left each one a sliver to hit.
	 */
	import { t } from '$lib/i18n';
	import Download from '@lucide/svelte/icons/download';
	import Ellipsis from '@lucide/svelte/icons/ellipsis';
	import House from '@lucide/svelte/icons/house';
	import Library from '@lucide/svelte/icons/library';
	import ListChecks from '@lucide/svelte/icons/list-checks';
	import Search from '@lucide/svelte/icons/search';
	import Settings from '@lucide/svelte/icons/settings';
	import Users from '@lucide/svelte/icons/users';
	import { page } from '$app/state';
	import { session } from '$lib/auth/session.svelte';
	import Modal from '$lib/components/primitives/Modal.svelte';
	import { socialBadge } from '$lib/stores/social.svelte';
	import { MOBILE_NAV, visibleNav } from './nav-items';

	const icons: Record<string, typeof House> = {
		'/': House,
		'/library': Library,
		'/list': ListChecks,
		'/social': Users,
		'/search': Search,
		'/acquire': Download,
		'/settings': Settings
	};
	const BAR = ['/', '/library', '/search', '/list'];
	const all = $derived(
		visibleNav(MOBILE_NAV, session.isAdmin).map((e) => ({ ...e, label: t(e.label), icon: icons[e.href] ?? House }))
	);
	const bar = $derived(BAR.map((href) => all.find((e) => e.href === href)).filter((e) => e !== undefined));
	const rest = $derived(all.filter((e) => !BAR.includes(e.href)));

	function active(href: string, exact: boolean): boolean {
		const path = page.url.pathname;
		return exact ? path === href : path === href || path.startsWith(href + '/');
	}
	const inRest = $derived(rest.some((e) => active(e.href, e.exact)));

	let more = $state(false);
	$effect(() => {
		void page.url.pathname;
		more = false;
	});
</script>

{#snippet tab(label: string, Icon: typeof House, on: boolean, dot: boolean)}
	<span class={['flex h-7 w-14 items-center justify-center rounded-full transition-colors', on ? 'bg-accent/15' : ''].join(' ')}>
		<span class="relative">
			<Icon class="size-[22px]" strokeWidth={on ? 2.2 : 1.8} />
			{#if dot}
				<span class="absolute -end-1 -top-0.5 size-2 rounded-full bg-accent"></span>
			{/if}
		</span>
	</span>
	<span class="text-[11px] font-medium leading-none">{label}</span>
{/snippet}

<nav
	class="fixed inset-x-0 bottom-0 z-40 border-t border-line bg-background/95 backdrop-blur md:hidden"
	aria-label={t('nav.primary')}
	style="padding-bottom: env(safe-area-inset-bottom);"
>
	<div class="grid" style:grid-template-columns="repeat({bar.length + 1}, minmax(0, 1fr))">
		{#each bar as item (item.href)}
			{@const on = active(item.href, item.exact)}
			<a
				href={item.href}
				aria-current={on ? 'page' : undefined}
				class={['flex h-[3.75rem] flex-col items-center justify-center gap-1.5 transition-colors', on ? 'text-accent' : 'text-muted active:text-foreground'].join(' ')}
			>
				{@render tab(item.label, item.icon, on, false)}
			</a>
		{/each}
		<button
			type="button"
			aria-haspopup="dialog"
			aria-expanded={more}
			class={['flex h-[3.75rem] flex-col items-center justify-center gap-1.5 transition-colors', inRest ? 'text-accent' : 'text-muted active:text-foreground'].join(' ')}
			onclick={() => (more = true)}
		>
			{@render tab('More', Ellipsis, inRest, socialBadge.unread > 0)}
		</button>
	</div>
</nav>

<Modal bind:open={more} title="More">
	<ul class="-mx-2">
		{#each rest as item (item.href)}
			{@const on = active(item.href, item.exact)}
			{@const Icon = item.icon}
			<li>
				<a
					href={item.href}
					aria-current={on ? 'page' : undefined}
					class={['flex min-h-14 items-center gap-4 rounded-lg px-3 text-[15px] font-medium', on ? 'bg-surface-active text-foreground' : 'text-foreground active:bg-surface-hover'].join(' ')}
				>
					<Icon class={['size-5', on ? 'text-accent' : 'text-muted'].join(' ')} />
					<span class="flex-1">{item.label}</span>
					{#if item.href === '/social' && socialBadge.unread > 0}
						<span class="rounded-full bg-accent/20 px-2 py-0.5 font-mono text-xs text-accent" aria-label={t('nav.unread', { count: socialBadge.unread })}>{socialBadge.unread}</span>
					{/if}
				</a>
			</li>
		{/each}
	</ul>
</Modal>
