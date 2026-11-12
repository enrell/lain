<script lang="ts">
	// Keys for programs that feed libraries (D-265): a downloader files
	// what it fetched in a library and tells lain what each file is.
	import { onMount } from 'svelte';
	import Check from '@lucide/svelte/icons/check';
	import Clipboard from '@lucide/svelte/icons/clipboard';
	import Trash2 from '@lucide/svelte/icons/trash-2';
	import { api } from '$lib/api';
	import type { ApiKey } from '$lib/api/list';
	import type { Library } from '$lib/api/types';
	import Button from '$lib/components/primitives/Button.svelte';
	import Modal from '$lib/components/primitives/Modal.svelte';
	import { focusSoon } from '$lib/components/social/focus';
	import Spinner from '$lib/components/primitives/Spinner.svelte';
	import SettingRow from '$lib/components/settings/SettingRow.svelte';
	import SettingsGroup from '$lib/components/settings/SettingsGroup.svelte';
	import { t } from '$lib/i18n';
	import { toasts } from '$lib/stores/toasts.svelte';
	import { errorMessage } from '$lib/utilities/errors';

	let ready = $state(false);
	let keys = $state<ApiKey[]>([]);
	let libraries = $state<Library[]>([]);
	let name = $state('');
	let picked = $state<string[]>([]);
	let creating = $state(false);
	// The key just made: shown until the page is left, never fetched again.
	let fresh = $state<ApiKey | null>(null);
	let copied = $state(false);

	const libraryName = (id: string): string => libraries.find((l) => l.id === id)?.name ?? id;

	onMount(async () => {
		try {
			[keys, libraries] = await Promise.all([
				api.apiKeys.list().then((r) => r.integrations),
				api.libraries.list()
			]);
		} catch (err) {
			toasts.error(errorMessage(err, 'Could not load the keys.'));
		} finally {
			ready = true;
		}
	});

	function toggle(id: string): void {
		picked = picked.includes(id) ? picked.filter((p) => p !== id) : [...picked, id];
	}

	async function create(): Promise<void> {
		creating = true;
		try {
			const made = await api.apiKeys.create({ name: name.trim(), libraries: picked });
			fresh = made;
			copied = false;
			keys = [...keys, { ...made, key: undefined }].sort((a, b) => a.name.localeCompare(b.name));
			name = '';
			picked = [];
		} catch (err) {
			toasts.error(errorMessage(err, 'Could not create the key.'));
		} finally {
			creating = false;
		}
	}

	// Revoking cannot be undone and breaks whatever uses the key: it is asked first.
	let revoking = $state<ApiKey | null>(null);
	let revokeOpen = $state(false);
	function askRevoke(key: ApiKey): void {
		revoking = key;
		revokeOpen = true;
		focusSoon(() => document.getElementById('confirm-revoke-key'));
	}

	async function remove(key: ApiKey): Promise<void> {
		revokeOpen = false;
		try {
			await api.apiKeys.remove(key.id);
			keys = keys.filter((k) => k.id !== key.id);
			if (fresh?.id === key.id) fresh = null;
			toasts.success(`Key “${key.name}” revoked.`);
		} catch (err) {
			toasts.error(errorMessage(err, 'Could not revoke the key.'));
		}
	}

	async function copy(): Promise<void> {
		if (!fresh?.key) return;
		try {
			await navigator.clipboard.writeText(fresh.key);
			copied = true;
		} catch {
			toasts.error('Could not copy to the clipboard.');
		}
	}
</script>

<SettingsGroup id="api-keys" title="API keys" description={t('settings.groups.integrations.apiKeys')}>
	{#if !ready}
		<div class="flex items-center gap-2 py-4 text-sm text-muted"><Spinner /> Loading…</div>
	{:else}
		{#if fresh?.key}
			<div class="bg-accent/5 py-4" role="status">
				<p class="text-sm text-foreground">
					Key for “{fresh.name}”. Copy it now: it is not shown again.
				</p>
				<div class="mt-2 flex items-center gap-2">
					<code class="min-w-0 flex-1 truncate rounded-md bg-field px-3 py-2 font-mono text-xs">{fresh.key}</code>
					<Button variant="tonal" size="field" onclick={() => void copy()}>
						{#if copied}<Check class="size-4" /> Copied{:else}<Clipboard class="size-4" /> Copy{/if}
					</Button>
				</div>
			</div>
		{/if}

		{#if keys.length > 0}
			<ul class="settings-items">
				{#each keys as key (key.id)}
					<li class="flex items-center gap-3 py-3">
						<div class="min-w-0 flex-1">
							<p class="truncate text-sm font-medium text-foreground">{key.name}</p>
							<p class="truncate text-xs text-muted">
								{key.libraries.map(libraryName).join(', ')}
							</p>
						</div>
						<Button variant="ghost" size="field" class="hover:text-danger!" onclick={() => askRevoke(key)} aria-label={`Revoke ${key.name}`}>
							<Trash2 class="size-4" /> Revoke
						</Button>
					</li>
				{/each}
			</ul>
		{/if}

		<form
			class="settings-items"
			onsubmit={(e) => {
				e.preventDefault();
				void create();
			}}
		>
			<SettingRow label="Name" hint={t('settings.groups.integrations.keyNameHint')}>
				<input
					class="h-9 w-full rounded-md border px-3 text-sm text-foreground outline-none placeholder:text-muted/60"
					bind:value={name}
					placeholder="Downloader"
					required
					aria-label="Name"
				/>
			</SettingRow>
			<SettingRow label="Libraries" hint={t('settings.groups.integrations.apiKeysScope')} stack>
				{#if libraries.length === 0}
					<p class="text-sm text-muted">Add a library first.</p>
				{:else}
					<div class="flex flex-wrap gap-2" role="group" aria-label="Libraries">
						{#each libraries as lib (lib.id)}
							<label
								class={[
									'flex h-9 cursor-pointer items-center gap-2 rounded-md px-3 text-sm transition-colors',
									picked.includes(lib.id) ? 'bg-accent/15 text-foreground' : 'bg-field text-muted hover:text-foreground'
								].join(' ')}
							>
								<input
									type="checkbox"
									class="accent-current"
									checked={picked.includes(lib.id)}
									onchange={() => toggle(lib.id)}
								/>
								{lib.name}
							</label>
						{/each}
					</div>
				{/if}
			</SettingRow>
			<div class="flex justify-end py-3">
				<Button type="submit" size="field" loading={creating} disabled={!name.trim() || picked.length === 0}>
					Create key <kbd class="ms-1 font-mono text-[10px] opacity-70">Enter</kbd>
				</Button>
			</div>
		</form>
	{/if}
</SettingsGroup>

<Modal
	bind:open={revokeOpen}
	title={t('settings.groups.integrations.revokeTitle', { name: revoking?.name ?? '' })}
	description={t('settings.groups.integrations.revokeBody')}
>
	{#snippet footer()}
		<Button variant="ghost" onclick={() => (revokeOpen = false)}>{t('common.cancel')} <kbd class="ms-1 font-mono text-[10px] text-muted">Esc</kbd></Button>
		<Button id="confirm-revoke-key" variant="danger" onclick={() => revoking && void remove(revoking)}>
			{t('settings.groups.integrations.revoke')} <kbd class="ms-1 font-mono text-[10px] opacity-70">Enter</kbd>
		</Button>
	{/snippet}
</Modal>
