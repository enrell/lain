<script lang="ts">
	import { onMount } from 'svelte';
	import { api, type AcquireSettings, type AcquireView, type Profile, type SubtitleProvider } from '$lib/api';
	import Button from '$lib/components/primitives/Button.svelte';
	import Modal from '$lib/components/primitives/Modal.svelte';
	import ProfileDialog from '$lib/components/acquire/ProfileDialog.svelte';
	import SubtitleProviderDialog from '$lib/components/acquire/SubtitleProviderDialog.svelte';
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
	let profiles = $state<Profile[]>([]);
	let profileOpen = $state(false);
	let editingProfile = $state<Profile | null>(null);
	let deleteProfile = $state<Profile | null>(null);
	let deleteOpen = $state(false);
	let subProviders = $state<SubtitleProvider[]>([]);
	let subOpen = $state(false);
	let editingSub = $state<SubtitleProvider | null>(null);
	let deleteSub = $state<SubtitleProvider | null>(null);
	let deleteSubOpen = $state(false);

	async function loadSubProviders(): Promise<void> {
		try {
			subProviders = (await api.acquire.subtitleProviders()).providers;
		} catch (err) {
			toasts.error(errorMessage(err, t('acquire.subtitles.loadFailed')));
		}
	}

	function editSub(p: SubtitleProvider | null): void {
		editingSub = p;
		subOpen = true;
	}

	function askDeleteSub(p: SubtitleProvider): void {
		deleteSub = p;
		deleteSubOpen = true;
		requestAnimationFrame(() => requestAnimationFrame(() => document.getElementById('confirm-delete-subprovider')?.focus()));
	}

	async function removeSub(): Promise<void> {
		if (!deleteSub) return;
		try {
			await api.acquire.deleteSubtitleProvider(deleteSub.id);
			deleteSubOpen = false;
			await loadSubProviders();
		} catch (err) {
			toasts.error(errorMessage(err, t('acquire.subtitles.failed')));
		}
	}

	function subKey(e: KeyboardEvent, p: SubtitleProvider): void {
		if (e.ctrlKey || e.metaKey || e.altKey) return;
		const el = e.currentTarget as HTMLElement;
		if (e.key === 'e' || e.key === 'Enter') editSub(p);
		else if (e.key === 'Delete') askDeleteSub(p);
		else if (e.key === 'j' || e.key === 'ArrowDown') (el.nextElementSibling as HTMLElement | null)?.focus();
		else if (e.key === 'k' || e.key === 'ArrowUp') (el.previousElementSibling as HTMLElement | null)?.focus();
		else return;
		e.preventDefault();
	}

	async function loadProfiles(): Promise<void> {
		try {
			profiles = (await api.acquire.profiles()).profiles;
		} catch (err) {
			toasts.error(errorMessage(err, t('acquire.profiles.loadFailed')));
		}
	}

	function editProfile(p: Profile | null): void {
		editingProfile = p;
		profileOpen = true;
	}

	function askDeleteProfile(p: Profile): void {
		deleteProfile = p;
		deleteOpen = true;
		requestAnimationFrame(() => requestAnimationFrame(() => document.getElementById('confirm-delete-profile')?.focus()));
	}

	async function removeProfile(): Promise<void> {
		if (!deleteProfile) return;
		try {
			await api.acquire.deleteProfile(deleteProfile.id);
			deleteOpen = false;
			await loadProfiles();
		} catch (err) {
			toasts.error(errorMessage(err, t('acquire.profiles.failed')));
		}
	}

	function profileKey(e: KeyboardEvent, p: Profile): void {
		if (e.ctrlKey || e.metaKey || e.altKey) return;
		const el = e.currentTarget as HTMLElement;
		if (e.key === 'e' || e.key === 'Enter') editProfile(p);
		else if (e.key === 'Delete' && p.id !== 'default') askDeleteProfile(p);
		else if (e.key === 'j' || e.key === 'ArrowDown') (el.nextElementSibling as HTMLElement | null)?.focus();
		else if (e.key === 'k' || e.key === 'ArrowUp') (el.previousElementSibling as HTMLElement | null)?.focus();
		else return;
		e.preventDefault();
	}

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
			await Promise.all([loadProfiles(), loadSubProviders()]);
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

		<SettingsGroup id="automation" title={t('acquire.settings.automationGroup')}>
			<SettingRow id="acquire-automation" label={t('acquire.settings.automation')} hint={t('acquire.settings.automationHint')}>
				<Switch bare label={t('acquire.settings.automation')} bind:checked={draft.automation} />
			</SettingRow>
			<SettingRow id="acquire-rss" label={t('acquire.settings.rssMinutes')} hint={t('acquire.settings.rssMinutesHint')}>
				<input type="number" min="10" max="1440" class={num} aria-label={t('acquire.settings.rssMinutes')} bind:value={draft.rss_minutes} />
			</SettingRow>
			<SettingRow id="acquire-search" label={t('acquire.settings.searchHours')} hint={t('acquire.settings.searchHoursHint')}>
				<input type="number" min="0" max="168" class={num} aria-label={t('acquire.settings.searchHours')} bind:value={draft.search_hours} />
			</SettingRow>
			<SettingRow id="acquire-stall" label={t('acquire.settings.stallHours')} hint={t('acquire.settings.stallHoursHint')}>
				<input type="number" min="0" max="168" class={num} aria-label={t('acquire.settings.stallHours')} bind:value={draft.stall_hours} />
			</SettingRow>
			<SettingRow id="acquire-subtitle-hours" label={t('acquire.settings.subtitleHours')} hint={t('acquire.settings.subtitleHoursHint')}>
				<input type="number" min="0" max="720" class={num} aria-label={t('acquire.settings.subtitleHours')} bind:value={draft.subtitle_hours} />
			</SettingRow>
		</SettingsGroup>

		<SettingsGroup id="profiles" title={t('acquire.profiles.group')}>
			{#snippet aside()}
				<Button size="sm" variant="secondary" onclick={() => editProfile(null)}>{t('acquire.profiles.add')}</Button>
			{/snippet}
			<p class="pt-3 text-xs text-muted">{t('acquire.profiles.keys')}</p>
			<div class="divide-y divide-hairline" role="grid" aria-label={t('acquire.profiles.group')}>
				{#each profiles as p (p.id)}
					<div role="row" tabindex="0" onkeydown={(e) => profileKey(e, p)} class="flex flex-wrap items-center justify-between gap-3 py-3 outline-none focus-visible:bg-surface-active/40">
						<div class="min-w-0" role="gridcell">
							<p class="text-sm font-medium text-foreground">{p.name}</p>
							<p class="font-mono text-[10px] text-muted">{t('acquire.profiles.summary', { resolutions: p.resolutions.join(' › '), cutoff: p.cutoff, seeders: p.min_seeders })}</p>
						</div>
						<div class="flex gap-1.5" role="gridcell">
							<Button size="sm" variant="ghost" tabindex={-1} onclick={() => editProfile(p)}>{t('acquire.indexers.edit')} <kbd class="ms-1 font-mono text-[10px] text-muted">e</kbd></Button>
							{#if p.id !== 'default'}
								<Button size="sm" variant="ghost" tabindex={-1} onclick={() => askDeleteProfile(p)}>{t('acquire.indexers.remove')} <kbd class="ms-1 font-mono text-[10px] text-muted">Del</kbd></Button>
							{/if}
						</div>
					</div>
				{/each}
			</div>
		</SettingsGroup>

		<SettingsGroup id="subtitles" title={t('acquire.subtitles.group')}>
			{#snippet aside()}
				<Button size="sm" variant="secondary" onclick={() => editSub(null)}>{t('acquire.subtitles.add')}</Button>
			{/snippet}
			{#if subProviders.length === 0}
				<p class="py-3 text-xs text-muted">{t('acquire.subtitles.none')}</p>
			{:else}
				<p class="pt-3 text-xs text-muted">{t('acquire.subtitles.keys')}</p>
				<div class="divide-y divide-hairline" role="grid" aria-label={t('acquire.subtitles.group')}>
					{#each subProviders as p (p.id)}
						<div role="row" tabindex="0" onkeydown={(e) => subKey(e, p)} class="flex flex-wrap items-center justify-between gap-3 py-3 outline-none focus-visible:bg-surface-active/40">
							<div class="min-w-0" role="gridcell">
								<p class="text-sm font-medium text-foreground">{p.name}</p>
								<p class="font-mono text-[10px] text-muted">
									{t('acquire.subtitles.summary', { kind: p.kind, priority: p.priority })}
									· {p.username || t('acquire.subtitles.anonymous')}{p.enabled ? '' : ` · ${t('acquire.subtitles.disabled')}`}
								</p>
							</div>
							<div class="flex gap-1.5" role="gridcell">
								<Button size="sm" variant="ghost" tabindex={-1} onclick={() => editSub(p)}>{t('acquire.indexers.edit')} <kbd class="ms-1 font-mono text-[10px] text-muted">e</kbd></Button>
								<Button size="sm" variant="ghost" tabindex={-1} onclick={() => askDeleteSub(p)}>{t('acquire.subtitles.remove')} <kbd class="ms-1 font-mono text-[10px] text-muted">Del</kbd></Button>
							</div>
						</div>
					{/each}
				</div>
			{/if}
		</SettingsGroup>

		<StagedBar count={changeCount} {saving} onapply={() => void save()} ondiscard={() => view && (draft = { ...view.settings })} />
	{/if}
</div>

<ProfileDialog bind:open={profileOpen} editing={editingProfile} onsaved={() => void loadProfiles()} />

<SubtitleProviderDialog bind:open={subOpen} editing={editingSub} onsaved={() => void loadSubProviders()} />

<Modal bind:open={deleteSubOpen} title={t('acquire.subtitles.deleteTitle', { name: deleteSub?.name ?? '' })} description={t('acquire.subtitles.deleteBody')}>
	{#snippet footer()}
		<Button variant="ghost" onclick={() => (deleteSubOpen = false)}>{t('common.cancel')} <kbd class="ms-1 font-mono text-[10px] text-muted">Esc</kbd></Button>
		<Button id="confirm-delete-subprovider" variant="danger" onclick={() => void removeSub()}>{t('acquire.subtitles.remove')} <kbd class="ms-1 font-mono text-[10px] opacity-70">Enter</kbd></Button>
	{/snippet}
</Modal>

<Modal bind:open={deleteOpen} title={t('acquire.profiles.deleteTitle', { name: deleteProfile?.name ?? '' })} description={t('acquire.profiles.deleteBody')}>
	{#snippet footer()}
		<Button variant="ghost" onclick={() => (deleteOpen = false)}>{t('common.cancel')} <kbd class="ms-1 font-mono text-[10px] text-muted">Esc</kbd></Button>
		<Button id="confirm-delete-profile" variant="danger" onclick={() => void removeProfile()}>{t('acquire.indexers.remove')} <kbd class="ms-1 font-mono text-[10px] opacity-70">Enter</kbd></Button>
	{/snippet}
</Modal>
