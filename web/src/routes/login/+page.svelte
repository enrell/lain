<script lang="ts">
	import { onMount, tick } from 'svelte';
	import Eye from '@lucide/svelte/icons/eye';
	import EyeOff from '@lucide/svelte/icons/eye-off';
	import { goto } from '$app/navigation';
	import { session } from '$lib/auth/session.svelte';
	import { signInRefusal } from '$lib/auth/sign-in';
	import { t } from '$lib/i18n';
	import { errorMessage } from '$lib/utilities/errors';
	import Button from '$lib/components/primitives/Button.svelte';
	import Input from '$lib/components/primitives/Input.svelte';
	import SignalMark from '$lib/components/primitives/SignalMark.svelte';

	/*
	 * The server answers a wrong pair with 401 and a bare 'invalid
	 * credentials'. It is accurate and useless: it names neither field, rules
	 * out nothing and offers no next step, so the form says something a
	 * person can act on. A disabled account shares the 401 and gets its own
	 * words (signInRefusal).
	 */
	function signInError(err: unknown): string {
		const refusal = signInRefusal(err);
		if (refusal === 'disabled') return t('auth.login.disabled');
		if (refusal === 'mismatch') return t('auth.login.mismatch');
		return errorMessage(err, t('auth.login.failed'));
	}

	let username = $state('');
	let password = $state('');
	let showPassword = $state(false);
	let error = $state<string | null>(null);
	let busy = $state(false);

	// Keyboard-first: the name goes straight into its field.
	onMount(() => document.getElementById('login-username')?.focus());

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
			// Retyping the password is the usual fix: it is selected, ready.
			await tick();
			const field = document.getElementById('login-password') as HTMLInputElement | null;
			field?.focus();
			field?.select();
		} finally {
			busy = false;
		}
	}
	let host = $state('');
	$effect(() => {
		host = window.location.host;
	});
</script>

<svelte:head><title>{t('auth.login.pageTitle')}</title></svelte:head>

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
			<p class="text-sm text-muted">{t('auth.login.tagline')}</p>
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
			<!-- Fields with an edge of their own (Input's `edged`, shared with Setup):
			     on the card's surface a borderless field is hard to find. -->
			<Input
				id="login-username"
				edged
				label={t('auth.username')}
				bind:value={username}
				autocomplete="username"
				autocapitalize="none"
				spellcheck={false}
				required
			/>
			<Input
				id="login-password"
				edged
				label={t('auth.password')}
				type={showPassword ? 'text' : 'password'}
				bind:value={password}
				autocomplete="current-password"
				required
			>
				{#snippet end()}
					<button
						type="button"
						class="flex h-11 w-11 items-center justify-center rounded-md text-muted hover:text-foreground focus-visible:outline-2 focus-visible:outline-accent"
						onclick={() => (showPassword = !showPassword)}
						aria-pressed={showPassword}
						aria-label={showPassword ? t('auth.hidePassword') : t('auth.showPassword')}
					>
						{#if showPassword}<EyeOff class="size-4" aria-hidden="true" />{:else}<Eye class="size-4" aria-hidden="true" />{/if}
					</button>
				{/snippet}
			</Input>
			<Button type="submit" size="lg" class="w-full" loading={busy}>
				{t('auth.login.submit')}
			</Button>
		</form>
		<!-- The way back in without a mail server: an admin sets a new password (Settings → Users). -->
		<p class="mt-4 text-center text-xs text-muted">{t('auth.login.forgot')}</p>
		<!-- Which server this is: more than one is common among self-hosters. -->
		{#if host}
			<p class="mt-2 text-center font-mono text-[11px] text-muted">{host}</p>
		{/if}
	</div>
</div>
