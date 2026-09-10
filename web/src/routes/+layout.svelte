<script lang="ts">
	import '../app.css';
	import { onMount } from 'svelte';
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import AppShell from '$lib/components/navigation/AppShell.svelte';
	import BootScreen from '$lib/components/navigation/BootScreen.svelte';
	import Toaster from '$lib/components/primitives/Toaster.svelte';
	import Spinner from '$lib/components/primitives/Spinner.svelte';
	import { session } from '$lib/auth/session.svelte';
	import { isAdminRoute, routeRedirect } from '$lib/utilities/guards';

	let { children } = $props();

	onMount(() => {
		void session.bootstrap();
	});

	// Single guard for the whole app: first-run setup, login, admin.
	$effect(() => {
		const target = routeRedirect({
			ready: session.ready,
			setupRequired: session.setupRequired,
			authenticated: session.authenticated,
			admin: session.isAdmin,
			pathname: page.url.pathname
		});
		if (target) void goto(target);
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

{#if !session.ready}
	<BootScreen />
{:else if session.setupRequired}
	{@render children()}
{:else if !session.authenticated}
	{#if isAuthPage}
		{@render children()}
	{:else}
		<!-- Redirect to /login is already in flight. -->
		<div class="flex min-h-dvh items-center justify-center">
			<Spinner class="size-5 text-muted" label="Redirecting" />
		</div>
	{/if}
{:else if blockedAdmin}
	<div class="flex min-h-dvh items-center justify-center">
		<Spinner class="size-5 text-muted" label="Redirecting" />
	</div>
{:else if isAuthPage}
	{@render children()}
{:else if isPlayer}
	{@render children()}
{:else}
	<AppShell>
		{@render children()}
	</AppShell>
{/if}

<Toaster />
