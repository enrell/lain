<script lang="ts">
	import { onDestroy, onMount } from 'svelte';
	import { api, type DownloadJob, type DownloadSettings, type DownloadsView, type Library } from '$lib/api';
	import { ApiError } from '$lib/api';
	import Button from '$lib/components/primitives/Button.svelte';
	import Input from '$lib/components/primitives/Input.svelte';
	import Select from '$lib/components/primitives/Select.svelte';
	import Spinner from '$lib/components/primitives/Spinner.svelte';
	import SettingRow from '$lib/components/settings/SettingRow.svelte';
	import SettingsGroup from '$lib/components/settings/SettingsGroup.svelte';
	import StagedBar from '$lib/components/settings/StagedBar.svelte';
	import { t } from '$lib/i18n';
	import { toasts } from '$lib/stores/toasts.svelte';
	import { errorMessage } from '$lib/utilities/errors';
	import { formatBytes } from '$lib/utilities/format';
	import { fromGiB, isActive, progressRatio, settingsChanges, toGiB } from './downloads';

	/*
	 * Server downloads (docs/slices/reading-downloads.md). The queue and
	 * cleanup act immediately; the limits stage behind one Ctrl+S bar like
	 * every other server setting (D-087). Keyboard: `/` focuses the URL,
	 * and a focused row takes p (pause/resume), c (cancel), Delete (remove).
	 */
	let view = $state<DownloadsView | null>(null);
	let libs = $state<Library[]>([]);
	let url = $state('');
	let name = $state('');
	let target = $state('');
	let adding = $state(false);
	let cleaning = $state(false);
	let saving = $state(false);
	let draft = $state<DownloadSettings | null>(null);
	let urlInput = $state<HTMLInputElement | null>(null);
	let timer: ReturnType<typeof setTimeout> | null = null;

	const changeCount = $derived(view && draft ? settingsChanges(view.settings, draft) : 0);
	const targetOptions = $derived([
		{ value: '', label: t('downloads.queue.targetDir') },
		...libs.map((l) => ({ value: l.id, label: `${l.name} — ${l.path}` }))
	]);

	async function refresh(): Promise<void> {
		try {
			const next = await api.downloads.list();
			const firstLoad = view === null;
			view = next;
			if (firstLoad || changeCount === 0) draft = { ...next.settings };
		} catch (err) {
			toasts.error(errorMessage(err, t('downloads.limits.loadFailed')));
		}
		schedule();
	}

	// Poll fast while something moves, slowly otherwise.
	function schedule(): void {
		if (timer) clearTimeout(timer);
		const busy = view?.jobs.some((j) => isActive(j.state)) ?? false;
		timer = setTimeout(() => void refresh(), busy ? 1000 : 10000);
	}

	onMount(async () => {
		try {
			libs = await api.libraries.list();
		} catch {
			libs = [];
		}
		await refresh();
	});
	onDestroy(() => {
		if (timer) clearTimeout(timer);
	});

	function failure(err: unknown, fallback: string): string {
		if (err instanceof ApiError && err.code === 'quota-exceeded') return t('downloads.error.quotaExceeded');
		if (err instanceof ApiError && err.code === 'disk-full') return t('downloads.error.diskFull');
		return errorMessage(err, fallback);
	}

	async function add(): Promise<void> {
		if (!url.trim() || adding) return;
		adding = true;
		try {
			const job = await api.downloads.add({
				url: url.trim(),
				name: name.trim() || undefined,
				library_id: target || undefined
			});
			toasts.success(t('downloads.queue.added', { name: job.name }));
			url = '';
			name = '';
			await refresh();
		} catch (err) {
			toasts.error(failure(err, t('downloads.queue.addFailed')));
		} finally {
			adding = false;
		}
	}

	async function act(job: DownloadJob, action: 'pause' | 'resume' | 'cancel' | 'remove'): Promise<void> {
		try {
			if (action === 'remove') {
				await api.downloads.remove(job.id);
				toasts.success(t('downloads.queue.removed', { name: job.name }));
			} else {
				await api.downloads.action(job.id, action);
			}
			await refresh();
		} catch (err) {
			toasts.error(failure(err, t('downloads.queue.actionFailed')));
		}
	}

	function toggle(job: DownloadJob): void {
		if (job.state === 'queued' || job.state === 'running') void act(job, 'pause');
		else if (job.state === 'paused' || job.state === 'failed') void act(job, 'resume');
	}

	function rowKey(e: KeyboardEvent, job: DownloadJob): void {
		if (e.ctrlKey || e.metaKey || e.altKey) return;
		const row = e.currentTarget as HTMLElement;
		switch (e.key) {
			case 'p':
				e.preventDefault();
				toggle(job);
				break;
			case 'c':
				e.preventDefault();
				if (job.state !== 'done' && job.state !== 'canceled') void act(job, 'cancel');
				break;
			case 'Delete':
				e.preventDefault();
				if (!isActive(job.state)) void act(job, 'remove');
				break;
			case 'ArrowDown':
			case 'j':
				e.preventDefault();
				(row.nextElementSibling as HTMLElement | null)?.focus();
				break;
			case 'ArrowUp':
			case 'k':
				e.preventDefault();
				(row.previousElementSibling as HTMLElement | null)?.focus();
				break;
		}
	}

	function pageKey(e: KeyboardEvent): void {
		const el = e.target as HTMLElement | null;
		if (el && (['INPUT', 'TEXTAREA', 'SELECT'].includes(el.tagName) || el.isContentEditable)) return;
		if (e.key === '/' && !e.ctrlKey && !e.metaKey) {
			e.preventDefault();
			urlInput?.focus();
		}
	}

	async function cleanup(): Promise<void> {
		cleaning = true;
		try {
			const rep = await api.downloads.cleanup();
			toasts.success(t('downloads.storage.cleaned', { count: rep.parts, freed: formatBytes(rep.freed_bytes) || '0 B' }));
			await refresh();
		} catch (err) {
			toasts.error(errorMessage(err, t('downloads.storage.cleanupFailed')));
		} finally {
			cleaning = false;
		}
	}

	async function save(): Promise<void> {
		if (!draft) return;
		saving = true;
		try {
			const saved = await api.downloads.saveSettings(draft);
			draft = { ...saved };
			if (view) view.settings = saved;
			toasts.success(t('downloads.limits.saved'));
			await refresh();
		} catch (err) {
			toasts.error(errorMessage(err, t('downloads.limits.saveFailed')));
		} finally {
			saving = false;
		}
	}

	function discard(): void {
		if (view) draft = { ...view.settings };
	}

	function progressLabel(job: DownloadJob): string {
		if (job.total > 0) return t('downloads.progress', { done: formatBytes(job.bytes) || '0 B', total: formatBytes(job.total) });
		return t('downloads.progressUnknown', { done: formatBytes(job.bytes) || '0 B' });
	}

	function targetLabel(job: DownloadJob): string {
		const lib = libs.find((l) => l.id === job.library_id);
		return t('downloads.into', { target: lib ? lib.name : job.dir });
	}
</script>

<svelte:head><title>{t('downloads.title')}</title></svelte:head>
<svelte:window onkeydown={pageKey} />

<div class="max-w-4xl pb-24">
	{#if !view || !draft}
		<div class="flex justify-center py-10"><Spinner class="size-5 text-muted" /></div>
	{:else}
		<SettingsGroup id="queue" title={t('downloads.queue.group')}>
			<SettingRow label={t('downloads.queue.add')} hint={t('downloads.queue.addHint')} stack>
				<form
					class="grid w-full gap-3 sm:grid-cols-[minmax(0,2fr)_minmax(0,1fr)]"
					onsubmit={(e) => {
						e.preventDefault();
						void add();
					}}
				>
					<div class="sm:col-span-2">
						<label class="mb-1.5 flex items-center gap-2 text-sm font-medium text-muted" for="download-url">
							{t('downloads.queue.url')} <kbd class="font-mono text-[10px]">/</kbd>
						</label>
						<input
							id="download-url"
							bind:this={urlInput}
							bind:value={url}
							type="url"
							required
							spellcheck={false}
							autocomplete="off"
							placeholder={t('downloads.queue.urlPlaceholder')}
							class="h-10 w-full rounded-md border border-transparent bg-field px-3 font-mono text-xs text-foreground placeholder:text-muted/60 focus:border-accent/60 focus:outline-none"
						/>
					</div>
					<Input label={t('downloads.queue.name')} bind:value={name} placeholder={t('downloads.queue.namePlaceholder')} spellcheck={false} autocomplete="off" />
					<Select label={t('downloads.queue.target')} bind:value={target} options={targetOptions} />
					<div class="sm:col-span-2">
						<Button type="submit" loading={adding} disabled={!url.trim()}>
							{t('downloads.queue.submit')} <kbd class="ms-1 font-mono text-[10px] opacity-70">Enter</kbd>
						</Button>
					</div>
				</form>
			</SettingRow>

			{#if view.jobs.length === 0}
				<p class="py-6 text-sm text-muted">{t('downloads.queue.empty')}</p>
			{:else}
				<p class="pt-4 text-xs text-muted">
					{t('downloads.queue.keys', { pause: 'p', cancel: 'c', remove: 'Delete', focus: '/' })}
				</p>
				<div class="mt-2 divide-y divide-hairline" role="grid" aria-label={t('downloads.queue.group')}>
					{#each view.jobs as job (job.id)}
						<div
							role="row"
							tabindex="0"
							class="grid gap-x-6 gap-y-2 py-3 outline-none focus-visible:bg-surface-active/40 sm:grid-cols-[minmax(0,1fr)_auto] sm:items-center"
							onkeydown={(e) => rowKey(e, job)}
						>
							<div class="min-w-0" role="gridcell">
								<p class="truncate text-sm font-medium text-foreground" title={job.url}>{job.name}</p>
								<p class="mt-0.5 flex flex-wrap items-center gap-x-3 font-mono text-[10px] uppercase tracking-[0.14em] text-muted">
									<span class={job.state === 'failed' ? 'text-danger' : job.state === 'done' ? 'text-accent' : ''}>
										{t(`downloads.state.${job.state}`)}
									</span>
									<span>{progressLabel(job)}</span>
									<span class="truncate normal-case tracking-normal">{targetLabel(job)}</span>
								</p>
								{#if job.error}<p class="mt-1 text-xs text-danger">{job.error}</p>{/if}
								{#if job.state !== 'done' && job.state !== 'canceled'}
									<div class="mt-2 h-1 w-full overflow-hidden rounded-full bg-line" role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(progressRatio(job) * 100)}>
										<div class="h-full bg-accent transition-[width]" style:width="{progressRatio(job) * 100}%"></div>
									</div>
								{/if}
							</div>
							<div class="flex flex-wrap items-center gap-1.5" role="gridcell">
								{#if job.state === 'queued' || job.state === 'running'}
									<Button size="sm" variant="ghost" tabindex={-1} onclick={() => void act(job, 'pause')}>
										{t('downloads.action.pause')} <kbd class="ms-1 font-mono text-[10px] text-muted">p</kbd>
									</Button>
								{:else if job.state === 'paused' || job.state === 'failed'}
									<Button size="sm" variant="ghost" tabindex={-1} onclick={() => void act(job, 'resume')}>
										{job.state === 'failed' ? t('downloads.action.retry') : t('downloads.action.resume')}
										<kbd class="ms-1 font-mono text-[10px] text-muted">p</kbd>
									</Button>
								{/if}
								{#if job.state !== 'done' && job.state !== 'canceled'}
									<Button size="sm" variant="ghost" tabindex={-1} onclick={() => void act(job, 'cancel')}>
										{t('downloads.action.cancel')} <kbd class="ms-1 font-mono text-[10px] text-muted">c</kbd>
									</Button>
								{/if}
								{#if !isActive(job.state)}
									<Button size="sm" variant="ghost" tabindex={-1} onclick={() => void act(job, 'remove')}>
										{t('downloads.action.remove')} <kbd class="ms-1 font-mono text-[10px] text-muted">Del</kbd>
									</Button>
								{/if}
							</div>
						</div>
					{/each}
				</div>
			{/if}
		</SettingsGroup>

		<SettingsGroup id="storage" title={t('downloads.storage.group')}>
			<SettingRow label={t('downloads.storage.used')} hint={t('downloads.storage.usedHint')}>
				<span class="font-mono text-xs text-foreground">
					{view.usage.max_bytes > 0
						? t('downloads.storage.usedValue', { used: formatBytes(view.usage.used_bytes) || '0 B', max: formatBytes(view.usage.max_bytes) })
						: t('downloads.storage.usedUnlimited', { used: formatBytes(view.usage.used_bytes) || '0 B' })}
				</span>
			</SettingRow>
			<SettingRow label={t('downloads.storage.free')} hint={t('downloads.storage.freeHint')}>
				<span class="font-mono text-xs text-foreground">
					{view.usage.free_bytes < 0
						? t('downloads.storage.freeUnknown')
						: t('downloads.storage.freeValue', { free: formatBytes(view.usage.free_bytes) || '0 B', floor: formatBytes(view.usage.min_free_bytes) || '0 B' })}
				</span>
			</SettingRow>
			<SettingRow label={t('downloads.storage.cleanup')} hint={t('downloads.storage.cleanupHint')}>
				<Button variant="secondary" size="sm" loading={cleaning} onclick={() => void cleanup()}>
					{t('downloads.storage.cleanup')}
				</Button>
			</SettingRow>
		</SettingsGroup>

		<SettingsGroup id="limits" title={t('downloads.limits.group')}>
			<SettingRow label={t('downloads.limits.dir')} hint={t('downloads.limits.dirHint')}>
				<input
					class="h-9 w-full max-w-sm rounded-md border border-transparent bg-field px-3 font-mono text-xs text-foreground focus:border-accent/60 focus:outline-none"
					aria-label={t('downloads.limits.dir')}
					spellcheck={false}
					bind:value={draft.dir}
				/>
			</SettingRow>
			<SettingRow label={t('downloads.limits.max')} hint={t('downloads.limits.maxHint')}>
				<input
					type="number"
					min="0"
					step="1"
					class="h-9 w-32 rounded-md border border-transparent bg-field px-3 text-end font-mono text-xs text-foreground focus:border-accent/60 focus:outline-none"
					aria-label={t('downloads.limits.max')}
					value={toGiB(draft.max_bytes)}
					oninput={(e) => (draft!.max_bytes = fromGiB(e.currentTarget.value))}
				/>
			</SettingRow>
			<SettingRow label={t('downloads.limits.minFree')} hint={t('downloads.limits.minFreeHint')}>
				<input
					type="number"
					min="0"
					step="1"
					class="h-9 w-32 rounded-md border border-transparent bg-field px-3 text-end font-mono text-xs text-foreground focus:border-accent/60 focus:outline-none"
					aria-label={t('downloads.limits.minFree')}
					value={toGiB(draft.min_free_bytes)}
					oninput={(e) => (draft!.min_free_bytes = fromGiB(e.currentTarget.value))}
				/>
			</SettingRow>
			<SettingRow label={t('downloads.limits.concurrency')} hint={t('downloads.limits.concurrencyHint')}>
				<input
					type="number"
					min="1"
					max="8"
					class="h-9 w-24 rounded-md border border-transparent bg-field px-3 text-end font-mono text-xs text-foreground focus:border-accent/60 focus:outline-none"
					aria-label={t('downloads.limits.concurrency')}
					bind:value={draft.concurrency}
				/>
			</SettingRow>
			<SettingRow label={t('downloads.limits.keepDays')} hint={t('downloads.limits.keepDaysHint')}>
				<input
					type="number"
					min="0"
					class="h-9 w-24 rounded-md border border-transparent bg-field px-3 text-end font-mono text-xs text-foreground focus:border-accent/60 focus:outline-none"
					aria-label={t('downloads.limits.keepDays')}
					bind:value={draft.keep_finished_days}
				/>
			</SettingRow>
		</SettingsGroup>

		<StagedBar count={changeCount} {saving} onapply={() => void save()} ondiscard={discard} />
	{/if}
</div>
