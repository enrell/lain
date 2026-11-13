<script lang="ts">
	import '../app.css';
	import { onMount } from 'svelte';
	import { Tooltip } from 'bits-ui';
	import { afterNavigate, beforeNavigate, goto } from '$app/navigation';
	import { page } from '$app/state';
	import AppShell from '$lib/components/navigation/AppShell.svelte';
	import BootScreen from '$lib/components/navigation/BootScreen.svelte';
	import ReturnFocus from '$lib/components/navigation/ReturnFocus.svelte';
	import Toaster from '$lib/components/primitives/Toaster.svelte';
	import Spinner from '$lib/components/primitives/Spinner.svelte';
	import { session } from '$lib/auth/session.svelte';
	import { t } from '$lib/i18n';
	import { appearance } from '$lib/stores/appearance';
	import { isAdminRoute, routeRedirect } from '$lib/utilities/guards';
	import { keepLinksInside } from '$lib/demo/links';

	let { children } = $props();

	beforeNavigate((nav) => {
		if (import.meta.env.DEV) console.debug('[nav] before', nav.to?.url?.pathname, nav.type);
	});
	afterNavigate((nav) => {
		if (import.meta.env.DEV) console.debug('[nav] after', nav.to?.url?.pathname);
	});

	// The site's live demo routes by hash; see $lib/demo/links.
	onMount(() => {
		if (!import.meta.env.VITE_DEMO) return;
		const stopLinks = keepLinksInside();
		// Same-page fragment anchors (#main, #shelf-...) would otherwise replace the
		// active hash route (#/...) under SvelteKit's hash router.
		const onFragmentClick = (e: MouseEvent) => {
			if (e.defaultPrevented || e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return;
			const a = (e.target as Element | null)?.closest('a[href]');
			const href = a?.getAttribute('href');
			if (!href || !href.startsWith('#') || href.startsWith('#/')) return;
			const id = decodeURIComponent(href.slice(1));
			const target = id ? document.getElementById(id) : null;
			if (!target) return;
			e.preventDefault();
			target.scrollIntoView({ block: 'start' });
			if (target instanceof HTMLElement && target.hasAttribute('tabindex')) target.focus({ preventScroll: true });
		};
		document.addEventListener('click', onFragmentClick, true);
		return () => {
			stopLinks?.();
			document.removeEventListener('click', onFragmentClick, true);
		};
	});

	onMount(() => {
		appearance.start();
		void session.bootstrap();
		return () => appearance.stop();
	});

	// Single guard for the whole app: first-run setup, login, admin.
	$effect(() => {
		const can = {
			ready: session.ready,
			setupRequired: session.setupRequired,
			authenticated: session.authenticated,
			freshSetup: session.freshSetup,
			admin: session.isAdmin,
			pathname: page.url.pathname
		};
		const target = routeRedirect(can);
		if (import.meta.env.DEV) console.debug('[guard]', JSON.stringify({ ...can, target }));
		if (target) void goto(target);
	});

	// Once the welcome screen is left, the exception no longer applies:
	// returning to /setup while authenticated redirects to Home.
	$effect(() => {
		if (session.freshSetup && page.url.pathname !== '/setup') {
			if (import.meta.env.DEV) console.debug('[guard] acknowledge fresh setup');
			session.acknowledgeSetup();
		}
	});

	const isAuthPage = $derived(page.url.pathname === '/login' || page.url.pathname === '/setup');
	const isPlayer = $derived(
		page.url.pathname.startsWith('/player/') || page.url.pathname.startsWith('/read/')
	);
	const blockedAdmin = $derived(
		session.ready &&
			session.authenticated &&
			!session.isAdmin &&
			isAdminRoute(page.url.pathname)
	);
</script>

<Tooltip.Provider delayDuration={450}>
	{#if !session.ready}
		<BootScreen />
	{:else if session.setupRequired || isAuthPage}
		{#if session.setupRequired && !isAuthPage}
			<!-- Redirect to /setup is in flight; do not mount pages that
			     would fire authenticated requests before it lands. -->
			<div class="flex min-h-dvh items-center justify-center">
				<Spinner class="size-5 text-muted" label={t('boot.redirecting')} />
			</div>
		{:else}
			{@render children()}
		{/if}
	{:else if !session.authenticated || blockedAdmin}
		<div class="flex min-h-dvh items-center justify-center">
			<Spinner class="size-5 text-muted" label={t('boot.redirecting')} />
		</div>
	{:else if isPlayer}
		{@render children()}
	{:else}
		<AppShell>
			{@render children()}
		</AppShell>
	{/if}

	<Toaster />
	<!-- Here, not in the shell: the player and the reader leave the shell, and
	     Back from them must still find the card that opened them. -->
	<ReturnFocus />
</Tooltip.Provider>
