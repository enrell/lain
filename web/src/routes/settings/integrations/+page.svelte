<script lang="ts">
	import { onMount } from 'svelte';
	import Check from '@lucide/svelte/icons/check';
	import Clipboard from '@lucide/svelte/icons/clipboard';
	import { api, type IntegrationPlatform } from '$lib/api';
	import Button from '$lib/components/primitives/Button.svelte';
	import Input from '$lib/components/primitives/Input.svelte';
	import Spinner from '$lib/components/primitives/Spinner.svelte';
	import { toasts } from '$lib/stores/toasts.svelte';
	import { errorMessage } from '$lib/utilities/errors';

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
			toasts.error(errorMessage(err, 'Could not load integration settings.'));
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
			toasts.success('Integration settings saved.');
		} catch (err) {
			toasts.error(errorMessage(err, 'Could not save the integration settings.'));
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
			toasts.error('Could not copy to the clipboard.');
		}
	}
</script>

<svelte:head><title>Integrations — Settings — Lain</title></svelte:head>

<div class="grid gap-6 lg:grid-cols-2">
	<section class="rounded-card border border-line bg-surface/60 p-5 lg:col-span-2">
		<h2 class="text-base font-semibold text-foreground">AniList</h2>
		<p class="mt-1 max-w-2xl text-sm text-muted">
			One OAuth application for the whole server (D-080). Register it in the
			AniList developer settings, paste the client credentials below and set
			the redirect URL to exactly the callback shown here. Users can then link
			their own accounts from Settings → Account.
		</p>
		{#if !ready}
			<div class="mt-6 flex justify-center"><Spinner class="size-5 text-muted" /></div>
		{:else}
			<form
				class="mt-5 max-w-xl space-y-4"
				onsubmit={(e) => {
					e.preventDefault();
					void save();
				}}
			>
				<div>
					<label class="block text-sm font-medium text-foreground" for="anilist-callback"
						>Callback URL to register</label
					>
					<div class="mt-2 flex items-center gap-2">
						<input
							id="anilist-callback"
							class="w-full rounded-md border border-line bg-background p-2 font-mono text-xs text-foreground"
							type="text"
							readonly
							value={anilist?.callback_url ?? ''}
						/>
						<Button
							variant="secondary"
							type="button"
							class="shrink-0 !px-3"
							onclick={() => void copyCallback()}
							aria-label="Copy callback URL"
						>
							{#if copied}<Check class="size-4" />{:else}<Clipboard class="size-4" />{/if}
						</Button>
					</div>
					<p class="mt-2 text-xs text-muted">
						AniList requires an exact match. If the server runs behind a reverse proxy, this
						is the public URL users reach — not the internal bind address.
					</p>
				</div>
				<Input
					label="Client ID"
					bind:value={clientId}
					autocomplete="off"
					spellcheck={false}
					placeholder="The numeric id from the AniList app"
				/>
				<Input
					label="Client secret"
					type="password"
					bind:value={clientSecret}
					autocomplete="off"
					hint={anilist?.secret_set
						? 'A secret is stored — leave blank to keep it.'
						: 'Stored on the server only; it is never shown again.'}
				/>
				<Button type="submit" loading={saving}>Save</Button>
			</form>
		{/if}
	</section>
</div>
