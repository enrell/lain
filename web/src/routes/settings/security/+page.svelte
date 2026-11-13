<script lang="ts">
	import { api, ApiError } from '$lib/api';
	import { t } from '$lib/i18n';
	import { session } from '$lib/auth/session.svelte';
	import Button from '$lib/components/primitives/Button.svelte';
	import SettingRow from '$lib/components/settings/SettingRow.svelte';
	import SettingsGroup from '$lib/components/settings/SettingsGroup.svelte';
	import { toasts } from '$lib/stores/toasts.svelte';
	import { errorMessage } from '$lib/utilities/errors';

	let oldPassword = $state('');
	let newPassword = $state('');
	let confirm = $state('');
	let fieldErrors = $state<{ old?: string; new?: string; confirm?: string }>({});
	let oldInput = $state<HTMLInputElement>();
	let newInput = $state<HTMLInputElement>();
	let confirmInput = $state<HTMLInputElement>();
	let error = $state<string | null>(null);
	let busy = $state(false);

	async function submit(event: SubmitEvent): Promise<void> {
		event.preventDefault();
		if (busy) return;
		error = null;
		const next: typeof fieldErrors = {};
		if (newPassword.length < 8) next.new = t('security.tooShort');
		if (confirm !== newPassword) next.confirm = t('security.mismatch');
		fieldErrors = next;
		// The cursor goes to the first field to fix, where its message is read out.
		if (next.new) return void newInput?.focus();
		if (next.confirm) return void confirmInput?.focus();

		busy = true;
		try {
			await api.me.changePassword(oldPassword, newPassword);
			// Rotating the password bumps the token version: mint a fresh
			// session with the new password instead of getting 401'd.
			await session.login(session.user?.username ?? '', newPassword);
			oldPassword = '';
			newPassword = '';
			confirm = '';
			toasts.success(t('security.changed'));
		} catch (err) {
			// The server names a wrong current password with a code; a server
			// from before the code answered the bare "invalid credentials".
			const message = errorMessage(err, t('security.failed'));
			const wrong = err instanceof ApiError && (err.code === 'wrong-password' || (!err.code && err.message === 'invalid credentials'));
			if (wrong) {
				fieldErrors = { old: t('security.wrongCurrent') };
				oldInput?.select();
			} else error = message;
		} finally {
			busy = false;
		}
	}

	const field = 'h-9 w-full rounded-md border px-3 text-sm text-foreground outline-none';
</script>

<svelte:head><title>{t('security.pageTitle')}</title></svelte:head>

<form onsubmit={submit}>
	<SettingsGroup id="password" title={t('security.group')} description={t('settings.groups.security.password')}>
		{#if error}
			<p class="py-3 text-sm text-danger" role="alert">{error}</p>
		{/if}
		<SettingRow label={t('security.current')}>
			<input bind:this={oldInput} class={field} type="password" bind:value={oldPassword} autocomplete="current-password" required aria-label={t('security.current')} aria-invalid={fieldErrors.old ? 'true' : undefined} aria-describedby={fieldErrors.old ? 'pw-old-error' : undefined} />
			{#snippet below()}{#if fieldErrors.old}<p id="pw-old-error" class="text-xs text-danger sm:text-right">{fieldErrors.old}</p>{/if}{/snippet}
		</SettingRow>
		<SettingRow label={t('security.new')} hint={t('security.tooShort')}>
			<input bind:this={newInput} class={field} type="password" bind:value={newPassword} autocomplete="new-password" required aria-label={t('security.new')} aria-invalid={fieldErrors.new ? 'true' : undefined} aria-describedby={fieldErrors.new ? 'pw-new-error' : undefined} />
			{#snippet below()}{#if fieldErrors.new}<p id="pw-new-error" class="text-xs text-danger sm:text-right">{fieldErrors.new}</p>{/if}{/snippet}
		</SettingRow>
		<SettingRow label={t('security.confirm')}>
			<input bind:this={confirmInput} class={field} type="password" bind:value={confirm} autocomplete="new-password" required aria-label={t('security.confirm')} aria-invalid={fieldErrors.confirm ? 'true' : undefined} aria-describedby={fieldErrors.confirm ? 'pw-confirm-error' : undefined} />
			{#snippet below()}{#if fieldErrors.confirm}<p id="pw-confirm-error" class="text-xs text-danger sm:text-right">{fieldErrors.confirm}</p>{/if}{/snippet}
		</SettingRow>
		<div class="flex items-center justify-between gap-3 py-4">
			<p class="text-xs text-muted">{t('security.othersOut')}</p>
			<Button type="submit" size="field" loading={busy}>{t('security.submit')} <kbd class="ms-1 font-mono text-[10px] opacity-70">Enter</kbd></Button>
		</div>
	</SettingsGroup>
</form>
