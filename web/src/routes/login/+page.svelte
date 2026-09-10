<script lang="ts">
	import { goto } from '$app/navigation';
	import { session } from '$lib/auth/session.svelte';
	import { errorMessage } from '$lib/utilities/errors';
	import Button from '$lib/components/primitives/Button.svelte';
	import Input from '$lib/components/primitives/Input.svelte';
	import SignalMark from '$lib/components/primitives/SignalMark.svelte';

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
			error = errorMessage(err, 'Sign in failed.');
		} finally {
			busy = false;
		}
	}
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
			<Input
				label="Username"
				bind:value={username}
				autocomplete="username"
				autocapitalize="none"
				spellcheck={false}
				required
			/>
			<div class="space-y-1.5">
				<Input
					label="Password"
					type={showPassword ? 'text' : 'password'}
					bind:value={password}
					autocomplete="current-password"
					required
				/>
				<button
					type="button"
					class="text-xs text-muted hover:text-foreground"
					onclick={() => (showPassword = !showPassword)}
					aria-pressed={showPassword}
				>
					{showPassword ? 'Hide password' : 'Show password'}
				</button>
			</div>
			<Button type="submit" class="w-full" loading={busy}>
				Sign in
			</Button>
		</form>
	</div>
</div>
