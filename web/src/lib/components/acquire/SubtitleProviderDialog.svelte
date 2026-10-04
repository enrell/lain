<script lang="ts">
	/*
	 * Create or edit a subtitle provider account (A-30). Secrets are never
	 * sent back by the server: an empty field keeps the stored value.
	 */
	import { api, type SubtitleProvider, type SubtitleProviderInput } from '$lib/api';
	import Button from '$lib/components/primitives/Button.svelte';
	import Modal from '$lib/components/primitives/Modal.svelte';
	import Switch from '$lib/components/primitives/Switch.svelte';
	import { t } from '$lib/i18n';
	import { toasts } from '$lib/stores/toasts.svelte';
	import { errorMessage } from '$lib/utilities/errors';

	let {
		open = $bindable(false),
		editing = null,
		onsaved
	}: { open?: boolean; editing?: SubtitleProvider | null; onsaved: () => void } = $props();

	let name = $state('');
	let baseUrl = $state('');
	let apiKey = $state('');
	let username = $state('');
	let password = $state('');
	let enabled = $state(true);
	let priority = $state(0);
	let saving = $state(false);
	let nameInput = $state<HTMLInputElement>();

	$effect(() => {
		if (!open) return;
		const p = editing;
		name = p?.name ?? 'OpenSubtitles';
		baseUrl = p?.base_url ?? '';
		apiKey = '';
		username = p?.username ?? '';
		password = '';
		enabled = p?.enabled ?? true;
		priority = p?.priority ?? 0;
		requestAnimationFrame(() => requestAnimationFrame(() => nameInput?.focus()));
	});

	async function save(e: SubmitEvent): Promise<void> {
		e.preventDefault();
		saving = true;
		const body: SubtitleProviderInput = {
			name, kind: 'opensubtitles', base_url: baseUrl.trim(), api_key: apiKey, username: username.trim(),
			password, enabled, priority: Number(priority) || 0
		};
		try {
			if (editing) await api.acquire.updateSubtitleProvider(editing.id, body);
			else await api.acquire.createSubtitleProvider(body);
			open = false;
			toasts.success(t('acquire.subtitles.saved', { name }));
			onsaved();
		} catch (err) {
			toasts.error(errorMessage(err, t('acquire.subtitles.failed')));
		} finally {
			saving = false;
		}
	}

	const field = 'h-10 w-full rounded-md border border-transparent bg-field px-3 text-sm text-foreground placeholder:text-muted/60 focus:border-accent/60 focus:outline-none';
	const needsKey = $derived(!editing?.has_api_key && !apiKey.trim());
</script>

<Modal bind:open title={editing ? t('acquire.subtitles.editTitle', { name: editing.name }) : t('acquire.subtitles.addTitle')} description={t('acquire.subtitles.description')}>
	<form id="subtitle-provider-form" class="space-y-3" onsubmit={save}>
		<label class="block space-y-1.5"><span class="text-xs text-muted">{t('acquire.subtitles.name')}</span>
			<input bind:this={nameInput} bind:value={name} required maxlength="60" class={field} />
		</label>
		<label class="block space-y-1.5"><span class="text-xs text-muted">{t('acquire.subtitles.apiKey')}</span>
			<input bind:value={apiKey} type="password" autocomplete="off" spellcheck={false} class="{field} font-mono" placeholder={editing?.has_api_key ? '••••••••' : ''} />
			{#if editing?.has_api_key}<span class="block text-[11px] text-muted">{t('acquire.subtitles.secretStored')}</span>{/if}
		</label>
		<div class="grid grid-cols-2 gap-3">
			<label class="block space-y-1.5"><span class="text-xs text-muted">{t('acquire.subtitles.username')}</span>
				<input bind:value={username} autocomplete="off" spellcheck={false} class={field} />
			</label>
			<label class="block space-y-1.5"><span class="text-xs text-muted">{t('acquire.subtitles.password')}</span>
				<input bind:value={password} type="password" autocomplete="new-password" class={field} placeholder={editing?.has_password ? '••••••••' : ''} />
			</label>
		</div>
		<label class="block space-y-1.5"><span class="text-xs text-muted">{t('acquire.subtitles.baseUrl')}</span>
			<input bind:value={baseUrl} spellcheck={false} class="{field} font-mono" placeholder="https://api.opensubtitles.com/api/v1" />
			<span class="block text-[11px] text-muted">{t('acquire.subtitles.baseUrlHint')}</span>
		</label>
		<div class="grid grid-cols-2 items-end gap-3">
			<label class="block space-y-1.5"><span class="text-xs text-muted">{t('acquire.subtitles.priority')}</span>
				<input bind:value={priority} type="number" class="{field} font-mono" />
			</label>
			<Switch label={t('acquire.subtitles.enabled')} bind:checked={enabled} />
		</div>
	</form>
	{#snippet footer()}
		<Button variant="ghost" onclick={() => (open = false)}>{t('common.cancel')} <kbd class="ms-1 font-mono text-[10px] text-muted">Esc</kbd></Button>
		<Button type="submit" form="subtitle-provider-form" loading={saving} disabled={!name.trim() || needsKey}>{t('acquire.subtitles.save')} <kbd class="ms-1 font-mono text-[10px] opacity-70">Enter</kbd></Button>
	{/snippet}
</Modal>
