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
	import Input from '$lib/components/primitives/Input.svelte';
	import Spinner from '$lib/components/primitives/Spinner.svelte';
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

	async function remove(key: ApiKey): Promise<void> {
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

<section id="api-keys" class="settings-section scroll-mt-28 !border-t-0 !pt-0">
	<h2 class="settings-heading">API keys</h2>
	<p class="mt-1 max-w-2xl text-sm text-muted">
		For a program that puts files in a library and tells Lain what they are. A key sees only the
		libraries picked here: their names and paths, nothing in them.
	</p>

	{#if !ready}
		<div class="mt-4 flex items-center gap-2 text-sm text-muted"><Spinner /> Loading…</div>
	{:else}
		{#if fresh?.key}
			<div class="mt-4 rounded-md border border-accent/40 bg-accent/5 p-3" role="status">
				<p class="text-sm text-foreground">
					Key for “{fresh.name}”. Copy it now: it is not shown again.
				</p>
				<div class="mt-2 flex items-center gap-2">
					<code class="min-w-0 flex-1 truncate rounded bg-field px-2 py-1.5 font-mono text-xs">{fresh.key}</code>
					<Button variant="secondary" size="sm" onclick={() => void copy()}>
						{#if copied}<Check class="size-4" /> Copied{:else}<Clipboard class="size-4" /> Copy{/if}
					</Button>
				</div>
			</div>
		{/if}

		{#if keys.length > 0}
			<ul class="settings-list mt-4">
				{#each keys as key (key.id)}
					<li class="flex items-center gap-3 py-2.5">
						<div class="min-w-0 flex-1">
							<p class="truncate text-sm font-medium text-foreground">{key.name}</p>
							<p class="truncate text-xs text-muted">
								{key.libraries.map(libraryName).join(', ')}
							</p>
						</div>
						<Button variant="ghost" size="sm" onclick={() => void remove(key)} aria-label={`Revoke ${key.name}`}>
							<Trash2 class="size-4" /> Revoke
						</Button>
					</li>
				{/each}
			</ul>
		{/if}

		<form
			class="mt-5 max-w-xl space-y-4"
			onsubmit={(e) => {
				e.preventDefault();
				void create();
			}}
		>
			<Input label="Name" bind:value={name} placeholder="Downloader" required />
			<fieldset>
				<legend class="mb-1.5 block text-sm font-medium text-muted">Libraries</legend>
				{#if libraries.length === 0}
					<p class="text-sm text-muted">Add a library first.</p>
				{:else}
					<div class="flex flex-wrap gap-2">
						{#each libraries as lib (lib.id)}
							<label
								class={[
									'flex cursor-pointer items-center gap-2 rounded-md border px-2.5 py-1.5 text-sm',
									picked.includes(lib.id) ? 'border-accent/60 text-foreground' : 'border-line text-muted'
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
			</fieldset>
			<Button type="submit" loading={creating} disabled={!name.trim() || picked.length === 0}>
				Create key <kbd class="ms-1 font-mono text-[10px] opacity-70">Enter</kbd>
			</Button>
		</form>
	{/if}
</section>
