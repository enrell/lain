<script lang="ts">
	import Check from '@lucide/svelte/icons/check';
	import ListPlus from '@lucide/svelte/icons/list-plus';
	import { untrack } from 'svelte';
	import { api, ApiError, type ListEntry, type ListEntryPatch, type TitleTrack } from '$lib/api';
	import Menu, { type MenuEntry } from '$lib/components/primitives/Menu.svelte';
	import { t } from '$lib/i18n';
	import { toasts } from '$lib/stores/toasts.svelte';
	import { usePageKeys } from '$lib/stores/page-keys.svelte';
	import { errorMessage } from '$lib/utilities/errors';
	import { isTypingTarget } from '$lib/utilities/guards';
	import { platformName, progressText, STATUSES, statusLabel } from '$lib/tracking/status';

	/*
	 * The title's entry on each linked list (D-289): its list, how far it
	 * is and the score, with the list and the score set from here and
	 * written back to the platform. A title the list neither holds nor can
	 * take (no AniList id in its metadata) shows nothing. Read from the server's stored copy, so
	 * the page never waits on AniList; a title the list lacks but whose
	 * metadata names its AniList id can be added. L opens the list menu.
	 */
	let { itemId, reading = false }: { itemId: string; reading?: boolean } = $props();

	let tracks = $state<TitleTrack[]>([]);
	let saving = $state('');
	let scores = $state<Record<string, string>>({});

	let loads = 0;
	async function load(id: string): Promise<void> {
		const mine = ++loads;
		try {
			const res = await api.list.tracking(id);
			if (mine !== loads) return;
			tracks = res.tracks;
			scores = Object.fromEntries(res.tracks.map((tr) => [tr.platform, tr.entry?.score ? String(tr.entry.score) : '']));
		} catch {
			// The list is extra: a title page without it is still whole.
			if (mine === loads) tracks = [];
		}
	}
	$effect(() => {
		const id = itemId;
		untrack(() => void load(id));
	});

	function failure(err: unknown, platform: string): string {
		const name = platformName(platform);
		if (err instanceof ApiError && err.code === 'rate-limited') return t('title.tracking.rateLimited', { platform: name });
		if (err instanceof ApiError && (err.code === 'token-invalid' || err.code === 'token-expired'))
			return t('title.tracking.reconnect', { platform: name });
		return errorMessage(err, t('title.tracking.failed', { platform: name }));
	}

	async function save(track: TitleTrack, patch: ListEntryPatch): Promise<void> {
		if (saving || !track.remote_id) return;
		saving = track.platform;
		try {
			const body = track.entry ? patch : { ...patch, item_id: itemId };
			const { entry } = await api.list.save(track.platform, track.remote_id, body);
			tracks = tracks.map((tr) => (tr.platform === track.platform ? { ...tr, entry: entry as ListEntry } : tr));
			scores = { ...scores, [track.platform]: entry.score ? String(entry.score) : '' };
			toasts.success(t('title.tracking.saved', { platform: platformName(track.platform) }));
		} catch (err) {
			toasts.error(failure(err, track.platform));
		} finally {
			saving = '';
		}
	}

	function statusEntries(track: TitleTrack): MenuEntry[] {
		return STATUSES.map((s) => ({
			label: statusLabel(s, reading),
			icon: track.entry?.status === s ? Check : undefined,
			disabled: saving !== '',
			onSelect: () => void save(track, { status: s })
		}));
	}

	function saveScore(track: TitleTrack): void {
		const raw = (scores[track.platform] ?? '').trim().replace(',', '.');
		const n = raw === '' ? 0 : Number(raw);
		if (!Number.isFinite(n) || n < 0 || n > 10) {
			toasts.error(t('title.tracking.scoreHint'));
			return;
		}
		if (n === (track.entry?.score ?? 0)) return;
		void save(track, { score: n });
	}

	const editable = $derived(tracks.filter((tr) => tr.entry || tr.remote_id));
	usePageKeys(() => (editable.length > 0 ? [{ keys: ['L'], label: t('pageKeys.titleTracking') }] : []));

	let root = $state<HTMLElement>();
	function onKey(e: KeyboardEvent): void {
		if (e.key !== 'l' || e.ctrlKey || e.metaKey || e.altKey || e.defaultPrevented) return;
		if (isTypingTarget(e.target) || document.querySelector('[role="dialog"], [role="menu"]')) return;
		const trigger = root?.querySelector<HTMLButtonElement>('[data-tracking-status] button');
		if (!trigger) return;
		e.preventDefault();
		trigger.focus();
		trigger.click();
	}
</script>

<svelte:window onkeydown={onKey} />

<!-- Only where the list has the title or can take it: a film AniList never heard of gets no row. -->
{#if editable.length > 0}
	<section bind:this={root} class="space-y-3" aria-label={t('title.tracking.label', { platform: platformName(editable[0].platform) })}>
		{#each editable as track (track.platform)}
			{@const name = platformName(track.platform)}
			<div class="flex flex-wrap items-center gap-x-4 gap-y-2 border-y border-hairline py-3">
				<p class="font-mono text-[11px] uppercase tracking-[0.16em] text-foreground/80">
					{t('title.tracking.label', { platform: name })}
				</p>
				{#if track.entry}
					<span data-tracking-status>
						<Menu label={statusLabel(track.entry.status, reading)} hint={t('title.tracking.statusHint', { platform: name })} entries={statusEntries(track)} variant="tonal" size="field" align="start" />
					</span>
					<span class="text-sm text-foreground tabular-nums">{progressText(track.entry)}</span>
					<label class="flex items-center gap-2 text-sm text-muted">
						{t('title.tracking.score')}
						<input
							type="number"
							inputmode="decimal"
							min="0"
							max="10"
							step="0.5"
							class="h-9 w-20 rounded-md border border-line bg-surface/60 px-2 text-sm text-foreground tabular-nums focus:border-accent/60 focus:outline-none"
							title={t('title.tracking.scoreHint')}
							aria-label={t('title.tracking.score')}
							disabled={saving !== ''}
							bind:value={scores[track.platform]}
							onkeydown={(e) => {
								if (e.key === 'Enter') {
									e.preventDefault();
									saveScore(track);
								}
							}}
						/>
					</label>
				{:else if track.remote_id}
					<span class="text-sm text-muted">{t('title.tracking.notListed')}</span>
					<span data-tracking-status>
						<Menu label={t('title.tracking.add', { platform: name })} entries={statusEntries(track)} variant="tonal" size="field" align="start">
							{#snippet icon()}<ListPlus class="size-4" aria-hidden="true" />{/snippet}
						</Menu>
					</span>
				{/if}
				<kbd class="max-md:hidden">L</kbd>
				<span class="ms-auto font-mono text-[10px] uppercase tracking-[0.14em] text-muted">
					{track.scrobble ? t(reading ? 'title.tracking.autoReading' : 'title.tracking.auto') : t('title.tracking.manual')}
				</span>
			</div>
		{/each}
	</section>
{/if}
