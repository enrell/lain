<script lang="ts">
	import { api } from '$lib/api';
	import { session } from '$lib/auth/session.svelte';
	import Button from '$lib/components/primitives/Button.svelte';
	import SettingRow from '$lib/components/settings/SettingRow.svelte';
	import SettingsGroup from '$lib/components/settings/SettingsGroup.svelte';
	import { toasts } from '$lib/stores/toasts.svelte';
	import { errorMessage } from '$lib/utilities/errors';

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

	const field = 'h-9 w-full rounded-md border px-2.5 text-sm text-foreground outline-none sm:w-72';
</script>

<svelte:head><title>Security — Settings — Lain</title></svelte:head>

<form onsubmit={submit}>
	<SettingsGroup id="password" title="Password">
		{#if error}
			<p class="border-b border-hairline py-3 text-sm text-danger" role="alert">{error}</p>
		{/if}
		<SettingRow label="Current password">
			<input class={field} type="password" bind:value={oldPassword} autocomplete="current-password" required aria-label="Current password" />
		</SettingRow>
		<SettingRow label="New password" hint="At least 8 characters.">
			<input class={field} type="password" bind:value={newPassword} autocomplete="new-password" required aria-label="New password" aria-invalid={fieldErrors.new ? 'true' : undefined} />
			{#snippet below()}{#if fieldErrors.new}<p class="text-xs text-danger sm:text-right">{fieldErrors.new}</p>{/if}{/snippet}
		</SettingRow>
		<SettingRow label="Confirm new password">
			<input class={field} type="password" bind:value={confirm} autocomplete="new-password" required aria-label="Confirm new password" aria-invalid={fieldErrors.confirm ? 'true' : undefined} />
			{#snippet below()}{#if fieldErrors.confirm}<p class="text-xs text-danger sm:text-right">{fieldErrors.confirm}</p>{/if}{/snippet}
		</SettingRow>
		<div class="flex items-center justify-between gap-3 py-4">
			<p class="text-xs text-muted">Every other session is signed out immediately.</p>
			<Button type="submit" size="sm" loading={busy}>Update password</Button>
		</div>
	</SettingsGroup>
</form>
