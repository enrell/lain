<script lang="ts">
	/*
	 * Subtitles for a monitored title, or for any video items (docs/slices/
	 * acquisition.md, Phase 3). Lists each video file, the profile
	 * languages it still lacks and its sidecar files. Enter on a file
	 * searches providers, Enter on an accepted subtitle fetches it, f
	 * fetches every missing one at once (monitored titles), Del on a
	 * sidecar moves it to the holding folder, l edits the languages to
	 * search. Backspace returns from a search to the file list.
	 */
	import { api, ApiError, type FileSubtitles, type SidecarInfo, type SubtitleChoice } from '$lib/api';
	import Button from '$lib/components/primitives/Button.svelte';
	import Modal from '$lib/components/primitives/Modal.svelte';
	import Spinner from '$lib/components/primitives/Spinner.svelte';
	import { i18n, t } from '$lib/i18n';
	import { toasts } from '$lib/stores/toasts.svelte';
	import { errorMessage } from '$lib/utilities/errors';

	let {
		open = $bindable(false),
		monitoredId = '',
		itemIds = [],
		title
	}: { open?: boolean; monitoredId?: string; itemIds?: string[]; title: string } = $props();

	let files = $state<FileSubtitles[] | null>(null);
	let profileLanguages = $state<string[]>([]);
	let languages = $state('');
	let file = $state<FileSubtitles | null>(null);
	let choices = $state<SubtitleChoice[] | null>(null);
	let busy = $state('');
	let pending = $state<SubtitleChoice | null>(null);
	let replaceOpen = $state(false);
	let removing = $state<{ file: FileSubtitles; sidecar: SidecarInfo } | null>(null);
	let removeOpen = $state(false);
	let list = $state<HTMLElement>();
	let languageInput = $state<HTMLInputElement>();

	const base = (p: string) => p.split(/[\\/]/).pop() ?? p;

	// Item mode reads one status per file; a few at a time, since each
	// one probes its file on the server.
	async function itemStatuses(ids: string[]): Promise<FileSubtitles[]> {
		const out: FileSubtitles[] = [];
		for (let i = 0; i < ids.length; i += 4) {
			const batch = await Promise.all(ids.slice(i, i + 4).map((id) => api.acquire.itemSubtitles(id)));
			for (const r of batch) {
				out.push(r.file);
				if (r.languages.length) profileLanguages = r.languages;
			}
		}
		return out;
	}

	async function load(): Promise<void> {
		try {
			files = monitoredId ? (await api.acquire.monitoredSubtitles(monitoredId)).files : await itemStatuses(itemIds);
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
		languages = '';
		void load().then(focusFirst);
	});

	async function search(f: FileSubtitles): Promise<void> {
		file = f;
		choices = null;
		try {
			choices = (await api.acquire.searchSubtitles(f.item_id, languages.trim() || f.missing.join(',') || undefined)).choices;
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
		if (!monitoredId) return;
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

	function askRemove(f: FileSubtitles, s: SidecarInfo): void {
		removing = { file: f, sidecar: s };
		removeOpen = true;
		requestAnimationFrame(() => requestAnimationFrame(() => document.getElementById('confirm-remove-sidecar')?.focus()));
	}

	async function remove(): Promise<void> {
		if (!removing) return;
		busy = 'remove';
		try {
			await api.acquire.removeSidecar(removing.file.item_id, removing.sidecar.name);
			toasts.success(t('acquire.subtitles.sidecarHeld', { name: removing.sidecar.name }));
			removeOpen = false;
			await load();
			focusFirst();
		} catch (err) {
			toasts.error(errorMessage(err, t('acquire.subtitles.removeFailed')));
		} finally {
			busy = '';
		}
	}

	function focusLanguages(): void {
		languageInput?.focus();
		languageInput?.select();
	}

	function languageKey(e: KeyboardEvent): void {
		if (e.key !== 'Enter') return;
		e.preventDefault();
		if (files?.length === 1) void search(files[0]);
		else focusFirst();
	}

	function move(el: HTMLElement, key: string): boolean {
		if (key === 'j' || key === 'ArrowDown') (el.nextElementSibling as HTMLElement | null)?.focus();
		else if (key === 'k' || key === 'ArrowUp') (el.previousElementSibling as HTMLElement | null)?.focus();
		else return false;
		return true;
	}

	function fileKey(e: KeyboardEvent, f: FileSubtitles, s?: SidecarInfo): void {
		if (e.ctrlKey || e.metaKey || e.altKey) return;
		if (e.key === 'Enter') void search(f);
		else if (e.key === 'f' && monitoredId) void fetchAll();
		else if (e.key === 'l') focusLanguages();
		else if (e.key === 'Delete' && s) askRemove(f, s);
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
	const field = 'h-9 w-full rounded-md border border-transparent bg-field px-3 font-mono text-sm text-foreground placeholder:text-muted/60 focus:border-accent/60 focus:outline-none';
	const sidecarTag = (s: SidecarInfo) => [s.tag || s.language, s.forced ? t('acquire.subtitles.forced') : '', s.hi ? t('acquire.subtitles.hi') : ''].filter(Boolean).join(' · ');
	const noLanguages = $derived(!monitoredId && !languages.trim() && profileLanguages.length === 0 && !!files?.length && files.every((f) => f.missing.length === 0));
</script>

<Modal bind:open title={file ? t('acquire.subtitles.searchFor', { file: base(file.path) }) : t('acquire.subtitles.panelTitle', { title })} description={t('acquire.subtitles.panelDescription')}>
	<div bind:this={list} class="max-h-[60vh] overflow-y-auto">
		{#if !file}
			<label class="mb-3 block space-y-1">
				<span class="flex items-center justify-between text-xs text-muted">{t('acquire.subtitles.languages')} <kbd class="font-mono text-[10px]">l</kbd></span>
				<input bind:this={languageInput} bind:value={languages} onkeydown={languageKey} spellcheck={false} autocomplete="off" class={field}
					placeholder={profileLanguages.length ? i18n.formatList(profileLanguages) : t('acquire.subtitles.languagesPlaceholder')} />
				<span class="block text-[11px] text-muted">{noLanguages ? t('acquire.subtitles.noLanguages') : t('acquire.subtitles.languagesHint')}</span>
			</label>
			{#if files === null}
				<div class="flex justify-center py-8"><Spinner class="size-5 text-muted" /></div>
			{:else if files.length === 0}
				<p class="py-4 text-sm text-muted">{t('acquire.subtitles.noFiles')}</p>
			{:else}
				<p class="pb-2 text-xs text-muted">{monitoredId ? t('acquire.subtitles.panelKeys') : t('acquire.subtitles.itemKeys')}</p>
				<div class="divide-y divide-hairline" role="grid" aria-label={t('acquire.subtitles.panelTitle', { title })}>
					{#each files as f (f.item_id)}
						<div role="row" tabindex="0" class={row} onkeydown={(e) => fileKey(e, f)}>
							<div class="min-w-0" role="gridcell">
								<p class="truncate text-sm text-foreground">{base(f.path)}</p>
								<p class="mt-0.5 font-mono text-[10px] text-muted">
									{#if f.missing.length}
										<span class="text-warning">{t('acquire.subtitles.missing', { languages: i18n.formatList(f.missing) })}</span>
									{:else}
										<span>{t('acquire.subtitles.nothingMissing')}</span>
									{/if}
								</p>
							</div>
							<Button size="sm" variant="ghost" tabindex={-1} onclick={() => void search(f)}>{t('acquire.wanted.search')} <kbd class="ms-1 font-mono text-[10px] text-muted">Enter</kbd></Button>
						</div>
						{#each f.sidecars as s (s.name)}
							<div role="row" tabindex="0" class="{row} ps-4" onkeydown={(e) => fileKey(e, f, s)}>
								<div class="min-w-0" role="gridcell">
									<p class="truncate font-mono text-[11px] text-muted" title={s.name}>{s.name}</p>
									<p class="{tag} mt-0.5 text-muted">{sidecarTag(s)} · {s.format}</p>
								</div>
								<Button size="sm" variant="ghost" tabindex={-1} onclick={() => askRemove(f, s)}>{t('acquire.subtitles.moveOut')} <kbd class="ms-1 font-mono text-[10px] text-muted">Del</kbd></Button>
							</div>
						{/each}
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
			{#if monitoredId}
				<Button loading={busy === 'all'} disabled={!files?.some((f) => f.missing.length)} onclick={() => void fetchAll()}>{t('acquire.subtitles.fetchAll')} <kbd class="ms-1 font-mono text-[10px] opacity-70">f</kbd></Button>
			{/if}
		{/if}
	{/snippet}
</Modal>

<Modal bind:open={replaceOpen} title={t('acquire.subtitles.replaceTitle')} description={t('acquire.subtitles.replaceBody')}>
	{#snippet footer()}
		<Button variant="ghost" onclick={() => (replaceOpen = false)}>{t('common.cancel')} <kbd class="ms-1 font-mono text-[10px] text-muted">Esc</kbd></Button>
		<Button id="confirm-replace-subtitle" onclick={() => pending && void take(pending, true)}>{t('acquire.subtitles.replace')} <kbd class="ms-1 font-mono text-[10px] opacity-70">Enter</kbd></Button>
	{/snippet}
</Modal>

<Modal bind:open={removeOpen} title={t('acquire.subtitles.moveOutTitle', { name: removing?.sidecar.name ?? '' })} description={t('acquire.subtitles.moveOutBody')}>
	{#snippet footer()}
		<Button variant="ghost" onclick={() => (removeOpen = false)}>{t('common.cancel')} <kbd class="ms-1 font-mono text-[10px] text-muted">Esc</kbd></Button>
		<Button id="confirm-remove-sidecar" loading={busy === 'remove'} onclick={() => void remove()}>{t('acquire.subtitles.moveOut')} <kbd class="ms-1 font-mono text-[10px] opacity-70">Enter</kbd></Button>
	{/snippet}
</Modal>
