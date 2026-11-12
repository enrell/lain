<script lang="ts">
	import Eye from '@lucide/svelte/icons/eye';
	import EyeOff from '@lucide/svelte/icons/eye-off';
	import { goto } from '$app/navigation';
	import { session } from '$lib/auth/session.svelte';
	import { errorMessage } from '$lib/utilities/errors';
	import Button from '$lib/components/primitives/Button.svelte';
	import SignalMark from '$lib/components/primitives/SignalMark.svelte';

	/*
	 * The server answers a wrong pair with a bare 'invalid credentials'. It is
	 * accurate and useless: it names neither field, rules out nothing and
	 * offers no next step, so the form says something a person can act on.
	 */
	function signInError(err: unknown): string {
		const message = errorMessage(err, 'Sign in failed.');
		return message === 'invalid credentials'
			? 'That username and password do not match. Check both and try again.'
			: message;
	}

	let username = $state('');
	let password = $state('');
	let showPassword = $state(false);
	let error = $state<string | null>(null);
	let busy = $state(false);

	async function submit(event: SubmitEvent): Promise<void> {
		event.preventDefault();
		if (busy) return;
		error = null;
		busy = true;
		try {
			await session.login(username.trim(), password);
			await goto('/');
		} catch (err) {
			error = signInError(err);
		} finally {
			busy = false;
		}
	}
	let host = $state('');
	$effect(() => {
		host = window.location.host;
	});
	// Keyboard first: the cursor waits in the username on arrival.
	let usernameInput = $state<HTMLInputElement>();
	$effect(() => {
		usernameInput?.focus();
	});
</script>

<svelte:head><title>Sign in — Lain</title></svelte:head>

<div class="relative flex min-h-dvh items-center justify-center overflow-hidden px-4">
	<div class="lattice absolute inset-0 opacity-30" aria-hidden="true"></div>
	<div
		class="absolute -top-40 left-1/2 size-96 -translate-x-1/2 rounded-full bg-accent/10 blur-3xl"
		aria-hidden="true"
	></div>

	<div class="relative w-full max-w-sm">
		<div class="mb-8 flex flex-col items-center gap-3 text-center">
			<SignalMark class="size-10 text-accent" />
			<h1 class="text-2xl font-semibold tracking-[0.24em] text-foreground">lain</h1>
			<p class="text-sm text-muted">Your media. Your machine. Your rules.</p>
		</div>

		<form class="space-y-4 rounded-card border border-line bg-surface/80 p-6" onsubmit={submit}>
			{#if error}
				<div
					class="rounded-md border border-danger/30 bg-danger/10 px-3 py-2 text-sm text-danger"
					role="alert"
				>
					{error}
				</div>
			{/if}
			<!-- Fields with an edge of their own: on the card's surface a
			     borderless field is hard to find. -->
			<div class="space-y-1.5">
				<label for="login-username" class="block text-sm font-medium text-muted">Username</label>
				<input
					id="login-username"
					bind:this={usernameInput}
					bind:value={username}
					autocomplete="username"
					autocapitalize="none"
					spellcheck={false}
					required
					class="h-11 w-full rounded-md border border-line bg-field px-3 text-sm text-foreground focus:border-accent focus:outline-none"
				/>
			</div>
			<div class="space-y-1.5">
				<label for="login-password" class="block text-sm font-medium text-muted">Password</label>
				<div class="relative">
					<input
						id="login-password"
						type={showPassword ? 'text' : 'password'}
						bind:value={password}
						autocomplete="current-password"
						required
						class="h-11 w-full rounded-md border border-line bg-field pl-3 pr-11 text-sm text-foreground focus:border-accent focus:outline-none"
					/>
					<button
						type="button"
						class="absolute inset-y-0 right-0 flex w-11 items-center justify-center rounded-md text-muted hover:text-foreground focus-visible:outline-2 focus-visible:outline-accent"
						onclick={() => (showPassword = !showPassword)}
						aria-pressed={showPassword}
						aria-label={showPassword ? 'Hide password' : 'Show password'}
					>
						{#if showPassword}<EyeOff class="size-4" aria-hidden="true" />{:else}<Eye class="size-4" aria-hidden="true" />{/if}
					</button>
				</div>
			</div>
			<Button type="submit" size="lg" class="w-full" loading={busy}>
				Sign in
			</Button>
		</form>
		<!-- Which server this is: more than one is common among self-hosters. -->
		{#if host}
			<p class="mt-4 text-center font-mono text-[11px] text-muted">{host}</p>
		{/if}
	</div>
</div>
