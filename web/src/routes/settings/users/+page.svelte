<script lang="ts">
	import { onMount } from 'svelte';
	import KeyRound from '@lucide/svelte/icons/key-round';
	import UserRound from '@lucide/svelte/icons/user-round';
	import Users from '@lucide/svelte/icons/users';
	import type { User } from '$lib/api/types';
	import { api } from '$lib/api';
	import { session } from '$lib/auth/session.svelte';
	import Badge from '$lib/components/primitives/Badge.svelte';
	import Button from '$lib/components/primitives/Button.svelte';
	import EmptyState from '$lib/components/primitives/EmptyState.svelte';
	import ErrorState from '$lib/components/primitives/ErrorState.svelte';
	import Input from '$lib/components/primitives/Input.svelte';
	import Modal from '$lib/components/primitives/Modal.svelte';
	import Select from '$lib/components/primitives/Select.svelte';
	import Spinner from '$lib/components/primitives/Spinner.svelte';
	import Switch from '$lib/components/primitives/Switch.svelte';
	import { toasts } from '$lib/stores/toasts.svelte';
	import { errorMessage } from '$lib/utilities/errors';
	import { formatDate } from '$lib/utilities/format';

	let users = $state<User[]>([]);
	let loading = $state(true);
	let error = $state<string | null>(null);
	let rowBusy = $state<string | null>(null);

	let createOpen = $state(false);
	let creating = $state(false);
	let formError = $state<string | null>(null);
	let form = $state({ username: '', password: '', role: 'user' });

	let resetTarget = $state<User | null>(null);
	let resetOpen = $state(false);
	let resetPassword = $state('');
	let resetting = $state(false);

	async function load(): Promise<void> {
		loading = true;
		error = null;
		try {
			users = await api.users.list();
		} catch (err) {
			error = errorMessage(err);
		} finally {
			loading = false;
		}
	}

	onMount(() => {
		void load();
	});

	function isSelf(user: User): boolean {
		return user.id === session.user?.id;
	}

	async function patch(user: User, input: Parameters<typeof api.users.patch>[1], message: string) {
		rowBusy = user.id;
		try {
			const updated = await api.users.patch(user.id, input);
			users = users.map((u) => (u.id === updated.id ? updated : u));
			toasts.success(message);
		} catch (err) {
			toasts.error(errorMessage(err, 'Could not update the user.'));
		} finally {
			rowBusy = null;
		}
	}

	async function createUser(event: SubmitEvent): Promise<void> {
		event.preventDefault();
		creating = true;
		formError = null;
		try {
			const user = await api.users.create({
				username: form.username.trim(),
				password: form.password,
				role: form.role as 'admin' | 'user'
			});
			users = [...users, user].sort((a, b) => a.username.localeCompare(b.username));
			createOpen = false;
			form = { username: '', password: '', role: 'user' };
			toasts.success(`User “${user.username}” created.`);
		} catch (err) {
			formError = errorMessage(err);
		} finally {
			creating = false;
		}
	}

	async function resetUserPassword(): Promise<void> {
		const target = resetTarget;
		if (!target) return;
		resetting = true;
		try {
			await api.users.patch(target.id, { password: resetPassword });
			resetOpen = false;
			resetPassword = '';
			resetTarget = null;
			toasts.success(`Password reset for “${target.username}”. Their sessions were signed out.`);
		} catch (err) {
			toasts.error(errorMessage(err, 'Could not reset the password.'));
		} finally {
			resetting = false;
		}
	}

	const roleOptions = [
		{ value: 'user', label: 'User' },
		{ value: 'admin', label: 'Admin' }
	];
</script>

<svelte:head><title>Users — Settings — Lain</title></svelte:head>

<div class="space-y-5">
	<div class="flex items-center justify-between gap-3">
		<p class="text-sm text-muted">
			{users.length} {users.length === 1 ? 'account' : 'accounts'}. Roles and disabling take effect
			on the next request.
		</p>
		<Button variant="secondary" size="sm" onclick={() => (createOpen = true)}>
			<Users class="size-4" /> Add user
		</Button>
	</div>

	{#if loading}
		<div class="h-24 animate-pulse rounded-card bg-surface-active/50"></div>
	{:else if error}
		<ErrorState message={error} retry={() => void load()} />
	{:else if users.length === 0}
		<EmptyState title="No users" description="This should not happen: setup creates the first admin.">
			{#snippet icon()}<UserRound class="size-6 text-muted" />{/snippet}
		</EmptyState>
	{:else}
		<ul class="divide-y divide-line overflow-hidden rounded-card border border-line">
			{#each users as user (user.id)}
				<li class="flex flex-wrap items-center gap-x-4 gap-y-3 bg-surface/40 px-4 py-3.5">
					<div class="flex min-w-0 flex-1 items-center gap-3">
						<span
							class="flex size-8 shrink-0 items-center justify-center rounded-full bg-surface-active text-xs font-semibold text-muted"
						>
							{user.username.slice(0, 1).toUpperCase()}
						</span>
						<div class="min-w-0">
							<div class="flex flex-wrap items-center gap-2">
								<p class="truncate font-medium text-foreground">{user.username}</p>
								{#if isSelf(user)}<Badge tone="accent">you</Badge>{/if}
								{#if user.disabled}<Badge tone="danger">disabled</Badge>{/if}
							</div>
							<p class="mt-0.5 text-xs text-muted">
								{user.role} · joined {formatDate(user.created_at)}
							</p>
						</div>
					</div>

					<div class="flex flex-wrap items-center gap-3">
						{#if rowBusy === user.id}
							<Spinner class="size-4 text-muted" />
						{/if}
						<div class="w-32">
							<Select
								aria-label={`Role for ${user.username}`}
								value={user.role}
								options={roleOptions}
								disabled={isSelf(user)}
								onValueChange={(role) => void patch(user, { role: role as 'admin' | 'user' }, `Role updated for ${user.username}.`)}
							/>
						</div>
						<div class="w-36">
							<Switch
								label="Active"
								checked={!user.disabled}
								disabled={isSelf(user)}
								onCheckedChange={(enabled) =>
									void patch(
										user,
										{ disabled: !enabled },
										enabled ? `${user.username} enabled.` : `${user.username} disabled.`
									)}
							/>
						</div>
						<Button
							variant="ghost"
							size="sm"
							onclick={() => {
								resetTarget = user;
								resetOpen = true;
							}}
						>
							<KeyRound class="size-4" /> Reset
						</Button>
					</div>
				</li>
			{/each}
		</ul>
	{/if}
</div>

<!-- Create -->
<Modal bind:open={createOpen} title="Add a user" description="Accounts are local to this server.">
	<form class="space-y-4" onsubmit={createUser}>
		{#if formError}
			<div
				class="rounded-md border border-danger/30 bg-danger/10 px-3 py-2 text-sm text-danger"
				role="alert"
			>
				{formError}
			</div>
		{/if}
		<Input label="Username" bind:value={form.username} autocapitalize="none" spellcheck={false} required />
		<Input
			label="Password"
			type="password"
			bind:value={form.password}
			hint="At least 8 characters."
			autocomplete="new-password"
			required
		/>
		<Select label="Role" bind:value={form.role} options={roleOptions} />
		<div class="flex justify-end gap-2 pt-1">
			<Button type="button" variant="ghost" onclick={() => (createOpen = false)}>Cancel</Button>
			<Button type="submit" loading={creating}>Create user</Button>
		</div>
	</form>
</Modal>

<!-- Reset password -->
<Modal
	bind:open={resetOpen}
	title="Reset password"
	description="The user's current sessions are signed out immediately."
>
	<form
		class="space-y-4"
		onsubmit={(e) => {
			e.preventDefault();
			void resetUserPassword();
		}}
	>
		<p class="text-sm text-muted">
			New password for <span class="font-medium text-foreground">{resetTarget?.username}</span>
		</p>
		<Input
			label="New password"
			type="password"
			bind:value={resetPassword}
			hint="At least 8 characters."
			autocomplete="new-password"
			required
		/>
		<div class="flex justify-end gap-2 pt-1">
			<Button type="button" variant="ghost" onclick={() => (resetOpen = false)}>Cancel</Button>
			<Button type="submit" loading={resetting} disabled={resetPassword.length < 8}>Reset password</Button>
		</div>
	</form>
</Modal>
