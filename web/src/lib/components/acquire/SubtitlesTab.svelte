<script lang="ts">
	/*
	 * Subtitle history (docs/slices/acquisition.md, A-35/A-40): the
	 * sidecars Lain wrote, and the provider files the sync check refused.
	 * A refused file is never taken again until cleared: Del on a focused
	 * refusal clears it, Shift+C clears them all; j/k move.
	 */
	import { onMount } from 'svelte';
	import { api, type SubtitleRecord, type SubtitleRefusal } from '$lib/api';
	import Button from '$lib/components/primitives/Button.svelte';
	import Spinner from '$lib/components/primitives/Spinner.svelte';
	import { t } from '$lib/i18n';
	import { toasts } from '$lib/stores/toasts.svelte';
	import { errorMessage } from '$lib/utilities/errors';
	import { formatRelative } from '$lib/utilities/format';
	import { isTypingTarget } from '$lib/utilities/guards';

	let written = $state<SubtitleRecord[] | null>(null);
	let refused = $state<SubtitleRefusal[]>([]);
	let providers = $state<Record<string, string>>({});

	const base = (p: string) => p.split(/[\\/]/).pop() ?? p;

	async function load(): Promise<void> {
		try {
			const [l, r, p] = await Promise.all([api.acquire.subtitleLedger(), api.acquire.subtitleRefusals(), api.acquire.subtitleProviders()]);
			written = l.subtitles;
			refused = r.refusals;
			providers = Object.fromEntries(p.providers.map((x) => [x.id, x.name]));
		} catch (err) {
			toasts.error(errorMessage(err, t('acquire.subtitles.loadLedgerFailed')));
			written = [];
		}
	}
	onMount(() => void load());

	async function clear(r: SubtitleRefusal, el?: HTMLElement): Promise<void> {
		const next = (el?.nextElementSibling ?? el?.previousElementSibling) as HTMLElement | null;
		try {
			await api.acquire.clearSubtitleRefusal(r.provider_id, r.file_id);
			await load();
			next?.focus();
		} catch (err) {
			toasts.error(errorMessage(err, t('acquire.queue.actionFailed')));
		}
	}

	async function clearAll(): Promise<void> {
		if (refused.length === 0) return;
		try {
			const r = await api.acquire.clearSubtitleRefusals();
			toasts.success(t('acquire.subtitles.cleared', { count: r.cleared }));
			await load();
		} catch (err) {
			toasts.error(errorMessage(err, t('acquire.queue.actionFailed')));
		}
	}

	function move(e: KeyboardEvent): boolean {
		const el = e.currentTarget as HTMLElement;
		if (e.key === 'j' || e.key === 'ArrowDown') (el.nextElementSibling as HTMLElement | null)?.focus();
		else if (e.key === 'k' || e.key === 'ArrowUp') (el.previousElementSibling as HTMLElement | null)?.focus();
		else return false;
		e.preventDefault();
		return true;
	}

	function refusalKey(e: KeyboardEvent, r: SubtitleRefusal): void {
		if (e.ctrlKey || e.metaKey || e.altKey || move(e)) return;
		if (e.key !== 'Delete') return;
		e.preventDefault();
		void clear(r, e.currentTarget as HTMLElement);
	}

	function writtenKey(e: KeyboardEvent): void {
		if (!e.ctrlKey && !e.metaKey && !e.altKey) move(e);
	}

	function pageKey(e: KeyboardEvent): void {
		if (e.key !== 'C' || e.ctrlKey || e.metaKey || e.altKey || isTypingTarget(e.target) || document.querySelector('[role="dialog"]')) return;
		e.preventDefault();
		void clearAll();
	}

	const heading = 'font-mono text-[10px] font-semibold uppercase tracking-[0.26em] text-muted';
	const row = 'flex flex-wrap items-center justify-between gap-3 py-2.5 outline-none focus-visible:bg-surface-active/40';
</script>

<svelte:window onkeydown={pageKey} />

<section class="space-y-2">
	<div class="flex items-center justify-between border-b border-hairline pb-2">
		<h2 class={heading}>{t('acquire.subtitles.tabRefused')}</h2>
		<Button size="sm" variant="ghost" disabled={refused.length === 0} onclick={() => void clearAll()}>{t('acquire.subtitles.clearAll')} <kbd class="ms-1 font-mono text-[10px] text-muted">Shift+C</kbd></Button>
	</div>
	<p class="text-xs text-muted">{t('acquire.subtitles.tabRefusedHint')}</p>
	{#if written === null}
		<div class="flex justify-center py-6"><Spinner class="size-5 text-muted" /></div>
	{:else if refused.length === 0}
		<p class="py-3 text-sm text-muted">{t('acquire.subtitles.tabRefusedEmpty')}</p>
	{:else}
		<p class="text-xs text-muted">{t('acquire.subtitles.tabKeys')}</p>
		<div class="divide-y divide-hairline" role="grid" aria-label={t('acquire.subtitles.tabRefused')}>
			{#each refused as r (r.provider_id + '\u0000' + r.file_id)}
				<div role="row" tabindex="0" onkeydown={(e) => refusalKey(e, r)} class={row}>
					<div class="min-w-0" role="gridcell">
						<p class="truncate text-sm text-foreground" title={r.media_path}>{r.media_path ? base(r.media_path) : t('acquire.subtitles.fileId', { id: r.file_id })}</p>
						<p class="font-mono text-[10px] text-muted">{r.reason}</p>
						<p class="font-mono text-[10px] text-muted/70">
							{providers[r.provider_id] ?? r.provider_id} · {t('acquire.subtitles.fileId', { id: r.file_id })}{#if r.at}{' · '}{formatRelative(r.at)}{/if}
						</p>
					</div>
					<Button size="sm" variant="ghost" tabindex={-1} onclick={() => void clear(r)}>{t('acquire.subtitles.clear')} <kbd class="ms-1 font-mono text-[10px] text-muted">Del</kbd></Button>
				</div>
			{/each}
		</div>
	{/if}
</section>

<section class="space-y-2 pt-6">
	<h2 class="border-b border-hairline pb-2 {heading}">{t('acquire.subtitles.tabWritten')}</h2>
	<p class="text-xs text-muted">{t('acquire.subtitles.tabWrittenHint')}</p>
	{#if written === null}
		<div class="flex justify-center py-6"><Spinner class="size-5 text-muted" /></div>
	{:else if written.length === 0}
		<p class="py-3 text-sm text-muted">{t('acquire.subtitles.tabWrittenEmpty')}</p>
	{:else}
		<div class="divide-y divide-hairline" role="grid" aria-label={t('acquire.subtitles.tabWritten')}>
			{#each written as s (s.path)}
				<div role="row" tabindex="0" onkeydown={writtenKey} class={row}>
					<div class="min-w-0" role="gridcell">
						<p class="truncate font-mono text-[11px] text-foreground" title={s.path}>{base(s.path)}</p>
						<p class="font-mono text-[10px] text-muted">
							{s.language} · {providers[s.provider_id] ?? s.provider_id}{#if s.hash_match}{' · '}<span class="text-accent">{t('acquire.subtitles.hash')}</span>{/if}{' · '}{formatRelative(s.at)}
						</p>
					</div>
				</div>
			{/each}
		</div>
	{/if}
</section>
