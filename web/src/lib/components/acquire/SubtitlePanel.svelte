<script lang="ts">
	/*
	 * Subtitles for one monitored title (docs/slices/acquisition.md,
	 * Phase 3). Lists the title's video files and the profile languages
	 * each still lacks; Enter on a file searches providers, Enter on an
	 * accepted subtitle fetches it, f fetches every missing one at once.
	 * Backspace returns from a search to the file list.
	 */
	import { api, ApiError, type FileSubtitles, type SubtitleChoice } from '$lib/api';
	import Button from '$lib/components/primitives/Button.svelte';
	import Modal from '$lib/components/primitives/Modal.svelte';
	import Spinner from '$lib/components/primitives/Spinner.svelte';
	import { i18n, t } from '$lib/i18n';
	import { toasts } from '$lib/stores/toasts.svelte';
	import { errorMessage } from '$lib/utilities/errors';

	let { open = $bindable(false), monitoredId, title }: { open?: boolean; monitoredId: string; title: string } = $props();

	let files = $state<FileSubtitles[] | null>(null);
	let file = $state<FileSubtitles | null>(null);
	let choices = $state<SubtitleChoice[] | null>(null);
	let busy = $state('');
	let pending = $state<SubtitleChoice | null>(null);
	let replaceOpen = $state(false);
	let list = $state<HTMLElement>();

	const base = (p: string) => p.split(/[\\/]/).pop() ?? p;

	async function load(): Promise<void> {
		try {
			files = (await api.acquire.monitoredSubtitles(monitoredId)).files;
		} catch (err) {
			toasts.error(errorMessage(err, t('acquire.subtitles.loadStatusFailed')));
			files = [];
		}
	}

	function focusFirst(): void {
		requestAnimationFrame(() => requestAnimationFrame(() => (list?.querySelector('[role="row"]') as HTMLElement | null)?.focus()));
	}

	$effect(() => {
		if (!open) return;
		files = null;
		file = null;
		choices = null;
		void load().then(focusFirst);
	});

	async function search(f: FileSubtitles): Promise<void> {
		file = f;
		choices = null;
		try {
			choices = (await api.acquire.searchSubtitles(f.item_id, f.missing.join(',') || undefined)).choices;
		} catch (err) {
			toasts.error(errorMessage(err, t('acquire.subtitles.searchFailed')));
			choices = [];
		}
		focusFirst();
	}

	function back(): void {
		file = null;
		choices = null;
		focusFirst();
	}

	async function take(c: SubtitleChoice, replace = false): Promise<void> {
		if (!file || !c.accepted) return;
		busy = c.provider_id + c.file_id;
		try {
			await api.acquire.downloadSubtitle(file.item_id, c, replace);
			toasts.success(t('acquire.subtitles.placed'));
			replaceOpen = false;
			await load();
			back();
		} catch (err) {
			if (err instanceof ApiError && err.status === 409 && !replace) {
				pending = c;
				replaceOpen = true;
				requestAnimationFrame(() => requestAnimationFrame(() => document.getElementById('confirm-replace-subtitle')?.focus()));
			} else {
				toasts.error(errorMessage(err, t('acquire.subtitles.downloadFailed')));
				if (choices) choices = choices.map((x) => (x === c ? { ...x, accepted: false, rejections: [errorMessage(err, '')] } : x));
			}
		} finally {
			busy = '';
		}
	}

	async function fetchAll(): Promise<void> {
		busy = 'all';
		try {
			const r = await api.acquire.fetchMonitoredSubtitles(monitoredId);
			toasts.success(t('acquire.subtitles.fetched', { count: r.written }));
			await load();
		} catch (err) {
			toasts.error(errorMessage(err, t('acquire.subtitles.downloadFailed')));
		} finally {
			busy = '';
		}
	}

	function move(el: HTMLElement, key: string): boolean {
		if (key === 'j' || key === 'ArrowDown') (el.nextElementSibling as HTMLElement | null)?.focus();
		else if (key === 'k' || key === 'ArrowUp') (el.previousElementSibling as HTMLElement | null)?.focus();
		else return false;
		return true;
	}

	function fileKey(e: KeyboardEvent, f: FileSubtitles): void {
		if (e.ctrlKey || e.metaKey || e.altKey) return;
		if (e.key === 'Enter') void search(f);
		else if (e.key === 'f') void fetchAll();
		else if (!move(e.currentTarget as HTMLElement, e.key)) return;
		e.preventDefault();
	}

	function choiceKey(e: KeyboardEvent, c: SubtitleChoice): void {
		if (e.ctrlKey || e.metaKey || e.altKey) return;
		if (e.key === 'Enter') void take(c);
		else if (e.key === 'Backspace') back();
		else if (!move(e.currentTarget as HTMLElement, e.key)) return;
		e.preventDefault();
	}

	const row = 'flex flex-wrap items-center justify-between gap-3 py-2.5 outline-none focus-visible:bg-surface-active/40';
	const tag = 'font-mono text-[9px] uppercase tracking-[0.18em]';
</script>

<Modal bind:open title={file ? t('acquire.subtitles.searchFor', { file: base(file.path) }) : t('acquire.subtitles.panelTitle', { title })} description={t('acquire.subtitles.panelDescription')}>
	<div bind:this={list} class="max-h-[60vh] overflow-y-auto">
		{#if !file}
			{#if files === null}
				<div class="flex justify-center py-8"><Spinner class="size-5 text-muted" /></div>
			{:else if files.length === 0}
				<p class="py-4 text-sm text-muted">{t('acquire.subtitles.noFiles')}</p>
			{:else}
				<p class="pb-2 text-xs text-muted">{t('acquire.subtitles.panelKeys')}</p>
				<div class="divide-y divide-hairline" role="grid" aria-label={t('acquire.subtitles.panelTitle', { title })}>
					{#each files as f (f.item_id)}
						<div role="row" tabindex="0" class={row} onkeydown={(e) => fileKey(e, f)}>
							<div class="min-w-0" role="gridcell">
								<p class="truncate text-sm text-foreground">{base(f.path)}</p>
								<p class="mt-0.5 flex flex-wrap gap-x-2 font-mono text-[10px] text-muted">
									{#if f.missing.length}
										<span class="text-warning">{t('acquire.subtitles.missing', { languages: i18n.formatList(f.missing) })}</span>
									{:else}
										<span>{t('acquire.subtitles.nothingMissing')}</span>
									{/if}
									{#each f.sidecars as s (s.name)}<span>{s.tag || s.language}{s.forced ? '·F' : ''}{s.hi ? '·SDH' : ''}</span>{/each}
								</p>
							</div>
							<Button size="sm" variant="ghost" tabindex={-1} onclick={() => void search(f)}>{t('acquire.wanted.search')} <kbd class="ms-1 font-mono text-[10px] text-muted">Enter</kbd></Button>
						</div>
					{/each}
				</div>
			{/if}
		{:else if choices === null}
			<div class="flex items-center justify-center gap-2 py-8 text-xs text-muted"><Spinner class="size-4" /> {t('acquire.subtitles.searching')}</div>
		{:else if choices.length === 0}
			<p class="py-4 text-sm text-muted">{t('acquire.subtitles.noChoices')}</p>
		{:else}
			<div class="divide-y divide-hairline" role="grid" aria-label={t('acquire.subtitles.searchFor', { file: base(file.path) })}>
				{#each choices as c, i (i)}
					<div role="row" tabindex="0" class="{row} {c.accepted ? '' : 'opacity-60'}" onkeydown={(e) => choiceKey(e, c)}>
						<div class="min-w-0" role="gridcell">
							<p class="flex flex-wrap items-baseline gap-x-2 text-sm text-foreground">
								<span class="font-mono">{c.region || c.language}</span>
								<span class="truncate">{c.release || c.file_name || c.file_id}</span>
							</p>
							<p class="mt-0.5 flex flex-wrap gap-x-2 text-muted">
								{#if c.hash_match}<span class="{tag} text-accent">{t('acquire.subtitles.hash')}</span>{/if}
								{#if c.hi}<span class={tag}>{t('acquire.subtitles.hi')}</span>{/if}
								{#if c.downloads}<span class="font-mono text-[10px]">{t('acquire.subtitles.downloads', { count: c.downloads })}</span>{/if}
								{#if !c.accepted}<span class="text-[11px] text-warning">{t('acquire.subtitles.rejected', { reasons: (c.rejections ?? []).join('; ') })}</span>{/if}
							</p>
						</div>
						{#if c.accepted}
							<Button size="sm" variant="ghost" tabindex={-1} loading={busy === c.provider_id + c.file_id} onclick={() => void take(c)}>{t('acquire.subtitles.take')} <kbd class="ms-1 font-mono text-[10px] text-muted">Enter</kbd></Button>
						{/if}
					</div>
				{/each}
			</div>
		{/if}
	</div>
	{#snippet footer()}
		{#if file}
			<Button variant="ghost" onclick={back}>{t('acquire.subtitles.back')} <kbd class="ms-1 font-mono text-[10px] text-muted">Backspace</kbd></Button>
		{:else}
			<Button variant="ghost" onclick={() => (open = false)}>{t('common.cancel')} <kbd class="ms-1 font-mono text-[10px] text-muted">Esc</kbd></Button>
			<Button loading={busy === 'all'} disabled={!files?.some((f) => f.missing.length)} onclick={() => void fetchAll()}>{t('acquire.subtitles.fetchAll')} <kbd class="ms-1 font-mono text-[10px] opacity-70">f</kbd></Button>
		{/if}
	{/snippet}
</Modal>

<Modal bind:open={replaceOpen} title={t('acquire.subtitles.replaceTitle')} description={t('acquire.subtitles.replaceBody')}>
	{#snippet footer()}
		<Button variant="ghost" onclick={() => (replaceOpen = false)}>{t('common.cancel')} <kbd class="ms-1 font-mono text-[10px] text-muted">Esc</kbd></Button>
		<Button id="confirm-replace-subtitle" onclick={() => pending && void take(pending, true)}>{t('acquire.subtitles.replace')} <kbd class="ms-1 font-mono text-[10px] opacity-70">Enter</kbd></Button>
	{/snippet}
</Modal>
