<script lang="ts">
	import { onMount } from 'svelte';
	import { api, type AcquireSettings, type AcquireView } from '$lib/api';
	import Select from '$lib/components/primitives/Select.svelte';
	import Spinner from '$lib/components/primitives/Spinner.svelte';
	import Switch from '$lib/components/primitives/Switch.svelte';
	import SettingRow from '$lib/components/settings/SettingRow.svelte';
	import SettingsGroup from '$lib/components/settings/SettingsGroup.svelte';
	import StagedBar from '$lib/components/settings/StagedBar.svelte';
	import { settingsChanges } from '$lib/acquire/format';
	import { t } from '$lib/i18n';
	import { toasts } from '$lib/stores/toasts.svelte';
	import { errorMessage } from '$lib/utilities/errors';
	import { formatBytes } from '$lib/utilities/format';

	/*
	 * Acquisition engine, seeding and import policy (docs/slices/
	 * acquisition.md, A-4/A-5/A-10). A SERVER page: changes stage behind
	 * Ctrl+S (D-087). Disk limits are the Downloads page's, shared.
	 */
	let view = $state<AcquireView | null>(null);
	let draft = $state<AcquireSettings | null>(null);
	let saving = $state(false);

	const changeCount = $derived(view && draft ? settingsChanges(view.settings, draft) : 0);
	const importOptions = $derived([
		{ value: 'hardlink', label: t('acquire.settings.importHardlink') },
		{ value: 'copy', label: t('acquire.settings.importCopy') },
		{ value: 'move', label: t('acquire.settings.importMove') }
	]);

	onMount(async () => {
		try {
			view = await api.acquire.settings();
			draft = { ...view.settings };
		} catch (err) {
			toasts.error(errorMessage(err, t('acquire.settings.loadFailed')));
		}
	});

	async function save(): Promise<void> {
		if (!draft) return;
		saving = true;
		try {
			view = await api.acquire.saveSettings(draft);
			draft = { ...view.settings };
			toasts.success(t('acquire.settings.saved'));
		} catch (err) {
			toasts.error(errorMessage(err, t('acquire.settings.saveFailed')));
		} finally {
			saving = false;
		}
	}

	const num = 'h-9 w-28 rounded-md border border-transparent bg-field px-3 text-end font-mono text-xs text-foreground focus:border-accent/60 focus:outline-none';
</script>

<svelte:head><title>{t('acquire.settings.title')}</title></svelte:head>

<div class="max-w-4xl pb-24">
	{#if !view || !draft}
		<div class="flex justify-center py-10"><Spinner class="size-5 text-muted" /></div>
	{:else}
		<SettingsGroup id="engine" title={t('acquire.settings.engineGroup')}>
			<SettingRow id="acquire-status" label={t('acquire.settings.status')} hint={t('acquire.settings.statusHint')}>
				<span class="font-mono text-xs text-foreground">
					{t('acquire.settings.statusValue', { port: String(view.port), used: formatBytes(view.usage.used_bytes) || '0 B' })}
				</span>
			</SettingRow>
			<SettingRow id="acquire-parser" label={t('acquire.settings.parser')} hint={t('acquire.settings.parserHint')}>
				<span class="font-mono text-[10px] uppercase tracking-[0.18em] {view.parser ? 'text-accent' : 'text-muted'}">
					{view.parser ? t('acquire.settings.parserOn') : t('acquire.settings.parserOff')}
				</span>
			</SettingRow>
			<SettingRow id="acquire-dir" label={t('acquire.settings.dir')} hint={t('acquire.settings.dirHint')}>
				<input class="h-9 w-full max-w-sm rounded-md border border-transparent bg-field px-3 font-mono text-xs text-foreground focus:border-accent/60 focus:outline-none" aria-label={t('acquire.settings.dir')} spellcheck={false} bind:value={draft.dir} />
			</SettingRow>
			<SettingRow id="acquire-port" label={t('acquire.settings.port')} hint={t('acquire.settings.portHint')}>
				<input type="number" min="0" max="65535" class={num} aria-label={t('acquire.settings.port')} bind:value={draft.listen_port} />
			</SettingRow>
			<SettingRow id="acquire-active" label={t('acquire.settings.maxActive')} hint={t('acquire.settings.maxActiveHint')}>
				<input type="number" min="1" max="20" class={num} aria-label={t('acquire.settings.maxActive')} bind:value={draft.max_active} />
			</SettingRow>
			<SettingRow id="acquire-peers" label={t('acquire.settings.maxPeers')} hint={t('acquire.settings.maxPeersHint')}>
				<input type="number" min="1" max="200" class={num} aria-label={t('acquire.settings.maxPeers')} bind:value={draft.max_peers} />
			</SettingRow>
			<SettingRow id="acquire-rates" label={t('acquire.settings.rates')} hint={t('acquire.settings.ratesHint')}>
				<label class="flex items-center gap-2 text-xs text-muted">
					{t('acquire.settings.down')}
					<input type="number" min="0" class={num} aria-label={t('acquire.settings.down')} bind:value={draft.download_kbps} />
				</label>
				<label class="flex items-center gap-2 text-xs text-muted">
					{t('acquire.settings.up')}
					<input type="number" min="0" class={num} aria-label={t('acquire.settings.up')} bind:value={draft.upload_kbps} />
				</label>
			</SettingRow>
		</SettingsGroup>

		<SettingsGroup id="seeding" title={t('acquire.settings.seedingGroup')}>
			<SettingRow id="acquire-ratio" label={t('acquire.settings.ratio')} hint={t('acquire.settings.ratioHint')}>
				<input type="number" min="0" max="100" step="0.1" class={num} aria-label={t('acquire.settings.ratio')} bind:value={draft.seed_ratio} />
			</SettingRow>
			<SettingRow id="acquire-minutes" label={t('acquire.settings.minutes')} hint={t('acquire.settings.minutesHint')}>
				<input type="number" min="0" class={num} aria-label={t('acquire.settings.minutes')} bind:value={draft.seed_minutes} />
			</SettingRow>
			<SettingRow id="acquire-remove" label={t('acquire.settings.removeAfter')} hint={t('acquire.settings.removeAfterHint')}>
				<Switch bare label={t('acquire.settings.removeAfter')} bind:checked={draft.remove_after_seeding} />
			</SettingRow>
		</SettingsGroup>

		<SettingsGroup id="import" title={t('acquire.settings.importGroup')}>
			<SettingRow id="acquire-import" label={t('acquire.settings.importMode')} hint={t('acquire.settings.importModeHint')}>
				<Select aria-label={t('acquire.settings.importMode')} options={importOptions} bind:value={draft.import_mode} />
			</SettingRow>
			<SettingRow id="acquire-budget" label={t('acquire.settings.budget')} hint={t('acquire.settings.budgetHint')}>
				<a href="/settings/downloads#limits" class="font-mono text-[10px] uppercase tracking-[0.2em] text-muted hover:text-foreground">
					{view.usage.max_bytes > 0
						? t('acquire.settings.budgetValue', { used: formatBytes(view.usage.used_bytes + view.usage.downloads_bytes) || '0 B', max: formatBytes(view.usage.max_bytes) })
						: t('acquire.settings.budgetUnlimited')}
				</a>
			</SettingRow>
		</SettingsGroup>

		<StagedBar count={changeCount} {saving} onapply={() => void save()} ondiscard={() => view && (draft = { ...view.settings })} />
	{/if}
</div>
