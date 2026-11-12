<script lang="ts">
	import { onMount, tick } from 'svelte';
	import CircleCheck from '@lucide/svelte/icons/circle-check';
	import FolderPlus from '@lucide/svelte/icons/folder-plus';
	import ArrowRight from '@lucide/svelte/icons/arrow-right';
	import { session } from '$lib/auth/session.svelte';
	import { t } from '$lib/i18n';
	import { errorMessage } from '$lib/utilities/errors';
	import Button from '$lib/components/primitives/Button.svelte';
	import LinkButton from '$lib/components/primitives/LinkButton.svelte';
	import Input from '$lib/components/primitives/Input.svelte';
	import SignalMark from '$lib/components/primitives/SignalMark.svelte';

	let step = $state<'account' | 'done'>('account');
	let username = $state('');
	let password = $state('');
	let confirm = $state('');
	let fieldErrors = $state<{ username?: string; password?: string; confirm?: string }>({});
	let error = $state<string | null>(null);
	let busy = $state(false);

	// The first screen of a new install: typing goes straight into the form,
	// as on the sign-in page.
	onMount(() => document.getElementById('setup-username')?.focus());

	function validate(): boolean {
		const next: typeof fieldErrors = {};
		if (username.trim().length < 1) next.username = t('auth.setup.pickUsername');
		if (password.length < 8) next.password = t('auth.setup.tooShort');
		if (confirm !== password) next.confirm = t('auth.setup.mismatch');
		fieldErrors = next;
		return Object.keys(next).length === 0;
	}

	async function submit(event: SubmitEvent): Promise<void> {
		event.preventDefault();
		if (busy) return;
		error = null;
		if (!validate()) {
			// Land on the first field to fix.
			const first = (['username', 'password', 'confirm'] as const).find((f) => fieldErrors[f]);
			if (first) document.getElementById(`setup-${first}`)?.focus();
			return;
		}
		busy = true;
		try {
			await session.setup(username.trim(), password);
			step = 'done';
			// The next step is one key away: Enter adds the first library.
			await tick();
			document.getElementById('setup-add-library')?.focus();
		} catch (err) {
			error = errorMessage(err, t('auth.setup.failed'));
		} finally {
			busy = false;
		}
	}

	// The sign-in page's fields: with an edge of their own, the card's
	// surface does not swallow them.
	const FIELD = 'h-11! border-line!';
</script>

<svelte:head><title>{t('auth.setup.pageTitle')}</title></svelte:head>

<div class="relative flex min-h-dvh items-center justify-center overflow-hidden px-4 py-10">
	<div class="lattice absolute inset-0 opacity-30" aria-hidden="true"></div>
	<div
		class="absolute -top-40 left-1/2 size-96 -translate-x-1/2 rounded-full bg-accent/10 blur-3xl"
		aria-hidden="true"
	></div>

	<div class="relative grid w-full max-w-3xl items-center gap-10 md:grid-cols-2">
		<div class="hidden md:block">
			<div class="flex items-center gap-3">
				<SignalMark class="size-9 text-accent" />
				<span class="text-2xl font-semibold tracking-[0.24em] text-foreground">lain</span>
			</div>
			<h1 class="mt-6 text-3xl font-semibold leading-tight tracking-tight text-foreground">
				{t('auth.setup.heading')}
			</h1>
			<p class="mt-3 text-sm leading-relaxed text-muted">{t('auth.setup.lead')}</p>
		</div>

		<div class="rounded-card border border-line bg-surface/80 p-6">
			{#if step === 'account'}
				<div class="mb-5 md:hidden">
					<div class="flex items-center gap-2.5">
						<SignalMark class="size-6 text-accent" />
						<span class="font-semibold tracking-[0.24em] text-foreground">lain</span>
					</div>
				</div>
				<h2 class="text-lg font-semibold text-foreground">{t('auth.setup.title')}</h2>
				<p class="mt-1 text-sm text-muted">{t('auth.setup.subtitle')}</p>
				<form class="mt-5 space-y-4" onsubmit={submit}>
					{#if error}
						<div
							class="rounded-md border border-danger/30 bg-danger/10 px-3 py-2 text-sm text-danger"
							role="alert"
						>
							{error}
						</div>
					{/if}
					<Input
						id="setup-username"
						label={t('auth.username')}
						bind:value={username}
						error={fieldErrors.username ?? null}
						autocomplete="username"
						autocapitalize="none"
						spellcheck={false}
						class={fieldErrors.username ? 'h-11!' : FIELD}
					/>
					<Input
						id="setup-password"
						label={t('auth.password')}
						type="password"
						bind:value={password}
						error={fieldErrors.password ?? null}
						hint={t('auth.setup.passwordHint')}
						autocomplete="new-password"
						class={fieldErrors.password ? 'h-11!' : FIELD}
					/>
					<Input
						id="setup-confirm"
						label={t('auth.setup.confirm')}
						type="password"
						bind:value={confirm}
						error={fieldErrors.confirm ?? null}
						autocomplete="new-password"
						class={fieldErrors.confirm ? 'h-11!' : FIELD}
					/>
					<Button type="submit" size="lg" class="w-full" loading={busy}>{t('auth.setup.submit')}</Button>
				</form>
			{:else}
				<div class="flex flex-col items-center py-4 text-center">
					<CircleCheck class="size-10 text-success" />
					<h2 class="mt-4 text-lg font-semibold text-foreground">{t('auth.setup.doneTitle')}</h2>
					<p class="mt-1 max-w-xs text-sm text-muted">{t('auth.setup.doneBody')}</p>
					<div class="mt-6 flex w-full flex-col gap-2">
						<LinkButton id="setup-add-library" href="/settings/libraries?add">
							<FolderPlus class="size-4" />
							{t('auth.setup.addLibrary')}
							<kbd class="ms-1 font-mono text-[10px] opacity-70">Enter</kbd>
						</LinkButton>
						<LinkButton href="/" variant="ghost">
							{t('auth.setup.explore')} <ArrowRight class="size-4" />
						</LinkButton>
					</div>
				</div>
			{/if}
		</div>
	</div>
</div>
