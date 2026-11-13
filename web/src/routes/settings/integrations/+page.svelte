<script lang="ts">
	import { onMount } from 'svelte';
	import Check from '@lucide/svelte/icons/check';
	import Clipboard from '@lucide/svelte/icons/clipboard';
	import { api, type IntegrationPlatform } from '$lib/api';
	import Button from '$lib/components/primitives/Button.svelte';
	import Spinner from '$lib/components/primitives/Spinner.svelte';
	import SettingRow from '$lib/components/settings/SettingRow.svelte';
	import SettingsGroup from '$lib/components/settings/SettingsGroup.svelte';
	import { t } from '$lib/i18n';
	import { toasts } from '$lib/stores/toasts.svelte';
	import { errorMessage } from '$lib/utilities/errors';
	import ApiKeys from './ApiKeys.svelte';

	// The operator's OAuth application credentials (D-080). The secret is
	// write-only: the server reports secret_set and keeps the stored one
	// when the field is left blank.
	let ready = $state(false);
	let saving = $state(false);
	let copied = $state(false);
	let anilist = $state<IntegrationPlatform | null>(null);
	let clientId = $state('');
	let clientSecret = $state('');

	onMount(async () => {
		try {
			const res = await api.integrations.settings();
			anilist = res.platforms['anilist'] ?? null;
			clientId = anilist?.client_id ?? '';
		} catch (err) {
			toasts.error(errorMessage(err, t('settings.integrationsPage.loadFailed')));
		} finally {
			ready = true;
		}
	});

	async function save(): Promise<void> {
		saving = true;
		try {
			const body: { anilist_client_id: string; anilist_client_secret?: string } = {
				anilist_client_id: clientId.trim()
			};
			if (clientSecret.trim() !== '') body.anilist_client_secret = clientSecret.trim();
			const res = await api.integrations.save(body);
			anilist = res.platforms['anilist'] ?? null;
			clientId = anilist?.client_id ?? '';
			clientSecret = '';
			toasts.success(t('settings.integrationsPage.saved'));
		} catch (err) {
			toasts.error(errorMessage(err, t('settings.integrationsPage.saveFailed')));
		} finally {
			saving = false;
		}
	}

	async function copyCallback(): Promise<void> {
		const url = anilist?.callback_url ?? '';
		if (!url) return;
		try {
			await navigator.clipboard.writeText(url);
			copied = true;
			setTimeout(() => (copied = false), 1500);
		} catch {
			toasts.error(t('settings.integrationsPage.copyFailed'));
		}
	}
</script>

<svelte:head><title>{t('settings.integrationsPage.pageTitle')}</title></svelte:head>

<form
	onsubmit={(e) => {
		e.preventDefault();
		void save();
	}}
>
	<SettingsGroup id="integrations" title="AniList" description={t('settings.groups.integrations.anilist')}>
		{#if !ready}
			<div class="flex justify-center py-8"><Spinner class="size-5 text-muted" /></div>
		{:else}
			<p class="py-4 text-sm leading-6 text-muted">{t('settings.groups.integrations.anilistSteps')}</p>
			<SettingRow
				label={t('settings.integrationsPage.callback')}
				hint={t('settings.groups.integrations.callbackHint')}
			>
				<div class="flex w-full items-center gap-2">
					<input
						id="anilist-callback"
						class="h-9 min-w-0 flex-1 rounded-md border px-3 font-mono text-xs text-foreground"
						type="text"
						readonly
						aria-label={t('settings.integrationsPage.callback')}
						value={anilist?.callback_url ?? ''}
					/>
					<Button
						variant="tonal"
						size="field"
						type="button"
						class="shrink-0"
						onclick={() => void copyCallback()}
						aria-label={t('settings.integrationsPage.copyCallback')}
						title={t('settings.integrationsPage.copyCallback')}
					>
						{#if copied}<Check class="size-4" />{:else}<Clipboard class="size-4" />{/if}
					</Button>
				</div>
			</SettingRow>
			<SettingRow label={t('settings.integrationsPage.clientId')} hint={t('settings.groups.integrations.clientIdHint')}>
				<input
					class="h-9 w-full rounded-md border px-3 font-mono text-sm text-foreground outline-none"
					bind:value={clientId}
					autocomplete="off"
					spellcheck={false}
					aria-label={t('settings.integrationsPage.clientId')}
				/>
			</SettingRow>
			<SettingRow
				label={t('settings.integrationsPage.clientSecret')}
				hint={anilist?.secret_set ? t('settings.integrationsPage.secretKept') : t('settings.integrationsPage.secretNew')}
			>
				<input
					class="h-9 w-full rounded-md border px-3 text-sm text-foreground outline-none"
					type="password"
					bind:value={clientSecret}
					autocomplete="off"
					aria-label={t('settings.integrationsPage.clientSecret')}
				/>
			</SettingRow>
			<div class="flex justify-end py-3">
				<Button type="submit" size="field" loading={saving}>{t('settings.integrationsPage.save')} <kbd class="ms-1 font-mono text-[10px] opacity-70">Enter</kbd></Button>
			</div>
		{/if}
	</SettingsGroup>
</form>
<ApiKeys />
