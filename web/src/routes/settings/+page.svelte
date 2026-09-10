<script lang="ts">
	import LogOut from '@lucide/svelte/icons/log-out';
	import ShieldCheck from '@lucide/svelte/icons/shield-check';
	import { goto } from '$app/navigation';
	import { api } from '$lib/api';
	import { session } from '$lib/auth/session.svelte';
	import Badge from '$lib/components/primitives/Badge.svelte';
	import Button from '$lib/components/primitives/Button.svelte';
	import Input from '$lib/components/primitives/Input.svelte';
	import { toasts } from '$lib/stores/toasts.svelte';
	import { errorMessage } from '$lib/utilities/errors';
	import { formatDate } from '$lib/utilities/format';

	let oldPassword = $state('');
	let newPassword = $state('');
	let confirm = $state('');
	let fieldErrors = $state<{ new?: string; confirm?: string }>({});
	let error = $state<string | null>(null);
	let busy = $state(false);

	async function submit(event: SubmitEvent): Promise<void> {
		event.preventDefault();
		if (busy) return;
		error = null;
		const next: typeof fieldErrors = {};
		if (newPassword.length < 8) next.new = 'At least 8 characters.';
		if (confirm !== newPassword) next.confirm = 'Passwords do not match.';
		fieldErrors = next;
		if (Object.keys(next).length > 0) return;

		busy = true;
		try {
			await api.me.changePassword(oldPassword, newPassword);
			// Rotating the password bumps the token version: mint a fresh
			// session with the new password instead of getting 401'd.
			await session.login(session.user?.username ?? '', newPassword);
			oldPassword = '';
			newPassword = '';
			confirm = '';
			toasts.success('Password changed. Other sessions were signed out.');
		} catch (err) {
			error = errorMessage(err, 'Could not change the password.');
		} finally {
			busy = false;
		}
	}

	function signOut(): void {
		session.logout();
		void goto('/login');
	}
</script>

<svelte:head><title>Account — Settings — Lain</title></svelte:head>

<div class="grid gap-6 lg:grid-cols-2">
	<section class="rounded-card border border-line bg-surface/60 p-5">
		<h2 class="text-base font-semibold text-foreground">Profile</h2>
		<dl class="mt-4 space-y-3 text-sm">
			<div class="flex items-center justify-between gap-4">
				<dt class="text-muted">Username</dt>
				<dd class="font-medium text-foreground">{session.user?.username}</dd>
			</div>
			<div class="flex items-center justify-between gap-4">
				<dt class="text-muted">Role</dt>
				<dd>
					<Badge tone={session.isAdmin ? 'accent' : 'neutral'}>
						{#if session.isAdmin}<ShieldCheck class="size-3" />{/if}
						{session.user?.role}
					</Badge>
				</dd>
			</div>
			{#if session.user?.created_at}
				<div class="flex items-center justify-between gap-4">
					<dt class="text-muted">Member since</dt>
					<dd class="text-foreground">{formatDate(session.user.created_at)}</dd>
				</div>
			{/if}
		</dl>
		<Button variant="secondary" class="mt-5" onclick={signOut}>
			<LogOut class="size-4" /> Sign out
		</Button>
	</section>

	<section class="rounded-card border border-line bg-surface/60 p-5">
		<h2 class="text-base font-semibold text-foreground">Change password</h2>
		<p class="mt-1 text-sm text-muted">
			Changing the password signs out every other session immediately.
		</p>
		<form class="mt-4 space-y-4" onsubmit={submit}>
			{#if error}
				<div
					class="rounded-md border border-danger/30 bg-danger/10 px-3 py-2 text-sm text-danger"
					role="alert"
				>
					{error}
				</div>
			{/if}
			<Input
				label="Current password"
				type="password"
				bind:value={oldPassword}
				autocomplete="current-password"
				required
			/>
			<Input
				label="New password"
				type="password"
				bind:value={newPassword}
				error={fieldErrors.new ?? null}
				hint="At least 8 characters."
				autocomplete="new-password"
				required
			/>
			<Input
				label="Confirm new password"
				type="password"
				bind:value={confirm}
				error={fieldErrors.confirm ?? null}
				autocomplete="new-password"
				required
			/>
			<Button type="submit" loading={busy}>Update password</Button>
		</form>
	</section>
</div>
