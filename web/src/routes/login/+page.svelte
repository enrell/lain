<script lang="ts">
	import { onMount, tick } from 'svelte';
	import Eye from '@lucide/svelte/icons/eye';
	import EyeOff from '@lucide/svelte/icons/eye-off';
	import X from '@lucide/svelte/icons/x';
	import { goto } from '$app/navigation';
	import { session } from '$lib/auth/session.svelte';
	import { signInRefusal, type SignInRefusal } from '$lib/auth/sign-in';
	import { forgetAccount, loadAccounts, type RememberedAccount } from '$lib/auth/storage';
	import { tileCommand } from '$lib/auth/tiles';
	import { t } from '$lib/i18n';
	import { errorMessage } from '$lib/utilities/errors';
	import Mascot from '$lib/components/profile/Mascot.svelte';
	import Button from '$lib/components/primitives/Button.svelte';
	import Input from '$lib/components/primitives/Input.svelte';
	import SignalMark from '$lib/components/primitives/SignalMark.svelte';

	/*
	 * The server answers a wrong pair with 401 and a bare 'invalid
	 * credentials'. It is accurate and useless: it names neither field, rules
	 * out nothing and offers no next step, so the form says something a
	 * person can act on. A disabled account shares the 401 and gets its own
	 * words (signInRefusal). A removed account is 404 unknown user: the
	 * tile stays until the person forgets it, and the form says why.
	 */
	function signInError(err: unknown): string {
		refusal = signInRefusal(err);
		if (refusal === 'disabled') return t('auth.login.disabled');
		if (refusal === 'mismatch') return t('auth.login.mismatch');
		if (refusal === 'unknown') return t('auth.login.unknown');
		return errorMessage(err, t('auth.login.failed'));
	}

	let username = $state('');
	let password = $state('');
	let showPassword = $state(false);
	let error = $state<string | null>(null);
	let refusal = $state<SignInRefusal | null>(null);
	let busy = $state(false);
	let accounts = $state<RememberedAccount[]>([]);
	let focused = $state(0);
	let tilesEl = $state<HTMLElement | null>(null);

	const gone = $derived(
		refusal === 'unknown' ? (accounts.find((a) => a.username === username.trim()) ?? null) : null
	);

	function initial(account: RememberedAccount): string {
		return (account.displayName || account.username).slice(0, 1).toUpperCase() || '?';
	}

	function focusTile(index: number): void {
		if (accounts.length === 0) return;
		focused = (index + accounts.length) % accounts.length;
		void tick().then(() => {
			tilesEl?.querySelectorAll<HTMLButtonElement>('[data-account]')[focused]?.focus();
		});
	}

	function pick(account: RememberedAccount): void {
		if (busy) return;
		username = account.username;
		password = '';
		error = null;
		refusal = null;
		void tick().then(() => {
			const field = document.getElementById('login-password') as HTMLInputElement | null;
			field?.focus();
		});
	}

	function forgetAt(index: number): void {
		const account = accounts[index];
		if (!account) return;
		forgetAccount(account.id);
		accounts = loadAccounts();
		if (accounts.length === 0) {
			document.getElementById('login-username')?.focus();
			return;
		}
		focusTile(Math.min(index, accounts.length - 1));
	}

	function onTilesKey(event: KeyboardEvent): void {
		const command = tileCommand(event.key, focused, accounts.length);
		if (!command || command.type === 'pick') return;
		event.preventDefault();
		if (command.type === 'move') focusTile(command.index);
		else if (command.type === 'forget') forgetAt(focused);
		else document.getElementById('login-username')?.focus();
	}

	// Tiles first when this browser remembers someone; otherwise the name field.
	onMount(async () => {
		accounts = loadAccounts();
		await tick();
		if (accounts.length > 0) focusTile(0);
		else document.getElementById('login-username')?.focus();
	});

	async function submit(event: SubmitEvent): Promise<void> {
		event.preventDefault();
		if (busy) return;
		error = null;
		refusal = null;
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

		{#if accounts.length > 0}
			<section class="mb-6" aria-labelledby="who-heading">
				<h2 id="who-heading" class="mb-3 text-center text-sm font-medium text-foreground">{t('auth.login.who')}</h2>
				<div
					bind:this={tilesEl}
					role="group"
					aria-labelledby="who-heading"
					class="flex flex-wrap justify-center gap-1"
				>
					{#each accounts as account, i (account.id)}
						<div class="relative w-24">
							<button
								type="button"
								data-account={account.id}
								class={[
									'flex w-full flex-col items-center gap-1.5 rounded-lg p-1.5 text-center outline-none focus-visible:ring-2 focus-visible:ring-accent',
									username.trim() === account.username ? 'bg-surface-hover' : 'hover:bg-surface-hover'
								].join(' ')}
								tabindex={i === focused ? 0 : -1}
								aria-label={t('auth.login.tile', { name: account.displayName, username: account.username })}
								onclick={() => pick(account)}
								onfocus={() => (focused = i)}
								onkeydown={onTilesKey}
							>
								<span
									aria-hidden="true"
									class="flex size-16 items-center justify-center overflow-hidden rounded-full bg-accent/15 ring-1 ring-white/10"
									style="container-type: size;"
								>
									{#if account.avatar.kind === 'mascot' && account.avatar.mascot}
										<Mascot id={account.avatar.mascot} class="size-full" />
									{:else}
										<span class="font-semibold text-accent" style="font-size: 45cqh;">{initial(account)}</span>
									{/if}
								</span>
								<span class="w-full truncate text-xs text-foreground">{account.displayName}</span>
							</button>
							<button
								type="button"
								class="absolute end-0.5 top-0.5 flex size-6 items-center justify-center rounded-full bg-surface text-muted ring-1 ring-line hover:text-foreground focus-visible:outline-2 focus-visible:outline-accent"
								tabindex={i === focused ? 0 : -1}
								aria-label={t('auth.login.forget', { name: account.displayName })}
								onclick={() => forgetAt(i)}
								onkeydown={onTilesKey}
							>
								<X class="size-3" aria-hidden="true" />
							</button>
						</div>
					{/each}
				</div>
				<p class="mt-3 text-center text-xs text-balance text-muted">
					{t('auth.login.tileKeys')} <kbd class="ms-1 font-mono">A</kbd>
				</p>
			</section>
		{/if}

		<form class="space-y-4 rounded-card border border-line bg-surface/80 p-6" onsubmit={submit}>
			{#if accounts.length > 0}
				<p class="text-center text-xs text-muted">{t('auth.login.orForm')}</p>
			{/if}
			{#if error}
				<div
					class="rounded-md border border-danger/30 bg-danger/10 px-3 py-2 text-sm text-danger"
					role="alert"
				>
					<p>{error}</p>
					{#if gone}
						<button
							type="button"
							class="mt-2 underline decoration-danger/40 underline-offset-2 focus-visible:outline-2 focus-visible:outline-accent"
							onclick={() => forgetAt(accounts.findIndex((a) => a.id === gone.id))}
						>
							{t('auth.login.forget', { name: gone.displayName })}
						</button>
					{/if}
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
		<!-- Balanced: in the card's width the sentence left one word on a line of its own. -->
		<p class="mt-4 text-center text-xs text-balance text-muted">{t('auth.login.forgot')}</p>
		<!-- Which server this is: more than one is common among self-hosters. -->
		{#if host}
			<p class="mt-2 text-center font-mono text-[11px] text-muted">{host}</p>
		{/if}
	</div>
</div>
