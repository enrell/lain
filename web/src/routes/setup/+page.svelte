<script lang="ts">
	import { goto } from '$app/navigation';
	import CircleCheck from '@lucide/svelte/icons/circle-check';
	import FolderPlus from '@lucide/svelte/icons/folder-plus';
	import ArrowRight from '@lucide/svelte/icons/arrow-right';
	import { session } from '$lib/auth/session.svelte';
	import { errorMessage } from '$lib/utilities/errors';
	import Button from '$lib/components/primitives/Button.svelte';
	import Input from '$lib/components/primitives/Input.svelte';
	import SignalMark from '$lib/components/primitives/SignalMark.svelte';

	let step = $state<'account' | 'done'>('account');
	let username = $state('');
	let password = $state('');
	let confirm = $state('');
	let fieldErrors = $state<{ username?: string; password?: string; confirm?: string }>({});
	let error = $state<string | null>(null);
	let busy = $state(false);

	function validate(): boolean {
		const next: typeof fieldErrors = {};
		if (username.trim().length < 1) next.username = 'Pick a username.';
		if (password.length < 8) next.password = 'At least 8 characters.';
		if (confirm !== password) next.confirm = 'Passwords do not match.';
		fieldErrors = next;
		return Object.keys(next).length === 0;
	}

	async function submit(event: SubmitEvent): Promise<void> {
		event.preventDefault();
		if (busy) return;
		error = null;
		if (!validate()) return;
		busy = true;
		try {
			await session.setup(username.trim(), password);
			step = 'done';
		} catch (err) {
			error = errorMessage(err, 'Could not create the account.');
		} finally {
			busy = false;
		}
	}
</script>

<svelte:head><title>Welcome — Lain</title></svelte:head>

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
				A quiet place for your media.
			</h1>
			<p class="mt-3 text-sm leading-relaxed text-muted">
				Lain indexes your files, streams them with byte-range seeking, and keeps everyone's
				progress separate. Everything runs on this machine.
			</p>
		</div>

		<div class="rounded-card border border-line bg-surface/80 p-6">
			{#if step === 'account'}
				<div class="mb-5 md:hidden">
					<div class="flex items-center gap-2.5">
						<SignalMark class="size-6 text-accent" />
						<span class="font-semibold tracking-[0.24em] text-foreground">lain</span>
					</div>
				</div>
				<h2 class="text-lg font-semibold text-foreground">Create your account</h2>
				<p class="mt-1 text-sm text-muted">
					The first account is the administrator. You can add more people later.
				</p>
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
						label="Username"
						bind:value={username}
						error={fieldErrors.username ?? null}
						autocomplete="username"
						autocapitalize="none"
						spellcheck={false}
					/>
					<Input
						label="Password"
						type="password"
						bind:value={password}
						error={fieldErrors.password ?? null}
						hint="At least 8 characters."
						autocomplete="new-password"
					/>
					<Input
						label="Confirm password"
						type="password"
						bind:value={confirm}
						error={fieldErrors.confirm ?? null}
						autocomplete="new-password"
					/>
					<Button type="submit" class="w-full" loading={busy}>Create account</Button>
				</form>
			{:else}
				<div class="flex flex-col items-center py-4 text-center">
					<CircleCheck class="size-10 text-success" />
					<h2 class="mt-4 text-lg font-semibold text-foreground">You're in.</h2>
					<p class="mt-1 max-w-xs text-sm text-muted">
						Next, point Lain at a directory on this server so it can index your files.
					</p>
					<div class="mt-6 flex w-full flex-col gap-2">
						<Button onclick={() => void goto('/settings/libraries')}>
							<FolderPlus class="size-4" /> Add a media library
						</Button>
						<Button variant="ghost" onclick={() => void goto('/')}>
							Explore Lain <ArrowRight class="size-4" />
						</Button>
					</div>
				</div>
			{/if}
		</div>
	</div>
</div>
