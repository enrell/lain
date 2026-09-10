<script lang="ts">
	import '../app.css';
	import { onMount } from 'svelte';
	import { Tooltip } from 'bits-ui';
	import { afterNavigate, beforeNavigate, goto } from '$app/navigation';
	import { page } from '$app/state';
	import AppShell from '$lib/components/navigation/AppShell.svelte';
	import BootScreen from '$lib/components/navigation/BootScreen.svelte';
	import Toaster from '$lib/components/primitives/Toaster.svelte';
	import Spinner from '$lib/components/primitives/Spinner.svelte';
	import { session } from '$lib/auth/session.svelte';
	import { isAdminRoute, routeRedirect } from '$lib/utilities/guards';

	let { children } = $props();

	beforeNavigate((nav) => {
		if (import.meta.env.DEV) console.debug('[nav] before', nav.to?.url?.pathname, nav.type);
	});
	afterNavigate((nav) => {
		if (import.meta.env.DEV) console.debug('[nav] after', nav.to?.url?.pathname);
	});

	onMount(() => {
		void session.bootstrap();
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
	const isPlayer = $derived(page.url.pathname.startsWith('/player/'));
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
				<Spinner class="size-5 text-muted" label="Redirecting" />
			</div>
		{:else}
			{@render children()}
		{/if}
	{:else if !session.authenticated || blockedAdmin}
		<div class="flex min-h-dvh items-center justify-center">
			<Spinner class="size-5 text-muted" label="Redirecting" />
		</div>
	{:else if isPlayer}
		{@render children()}
	{:else}
		<AppShell>
			{@render children()}
		</AppShell>
	{/if}

	<Toaster />
</Tooltip.Provider>
