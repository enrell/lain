<script lang="ts">
	import { onMount } from 'svelte';
	import ListChecks from '@lucide/svelte/icons/list-checks';
	import RefreshCw from '@lucide/svelte/icons/refresh-cw';
	import { goto } from '$app/navigation';
	import { api, type LinkedAccountView, type ListEntry } from '$lib/api';
	import { session } from '$lib/auth/session.svelte';
	import FirstRun from '$lib/components/media/FirstRun.svelte';
	import { framedGrid } from '$lib/components/media/grid';
	import Button from '$lib/components/primitives/Button.svelte';
	import EmptyState from '$lib/components/primitives/EmptyState.svelte';
	import LinkButton from '$lib/components/primitives/LinkButton.svelte';
	import ErrorState from '$lib/components/primitives/ErrorState.svelte';
	import Segmented from '$lib/components/primitives/Segmented.svelte';
	import Skeleton from '$lib/components/primitives/Skeleton.svelte';
	import Switch from '$lib/components/primitives/Switch.svelte';
	import ListEntryCard from '$lib/components/media/ListEntryCard.svelte';
	import { toasts } from '$lib/stores/toasts.svelte';
	import { errorMessage } from '$lib/utilities/errors';
	import { formatRelative } from '$lib/utilities/format';
	import { isTypingTarget } from '$lib/utilities/guards';
	import { usePageKeys } from '$lib/stores/page-keys.svelte';
	import { isReadingMedia, platformName, STATUSES, statusLabel } from '$lib/tracking/status';
	import { t } from '$lib/i18n';

	/*
	 * Tracking (D-078, D-288): the lists a linked account keeps (AniList's
	 * Watching, Planning, Completed, Paused, Dropped and Rewatching, anime
	 * and manga), each entry joined with the library: one the library holds
	 * opens its title, a planned one it lacks offers Acquire's search to an
	 * administrator. The strip over it says what lain last sent and holds
	 * the switch that makes watching and reading update the list.
	 *
	 * Keys: S syncs, A turns automatic updates on or off, M switches anime
	 * and manga, [ and ] step through the lists, I keeps what is in the
	 * library.
	 */
	type Media = 'anime' | 'manga' | 'other';

	let entries = $state<ListEntry[]>([]);
	let ready = $state(false);
	let loadError = $state('');
	let links = $state<LinkedAccountView[]>([]);
	let syncing = $state(false);
	let switching = $state(false);
	let media = $state<Media>('anime');
	let status = $state<string>('');
	let libraryOnly = $state(false);

	const mediaOf = (e: ListEntry): Media =>
		e.media_type === 'anime' ? 'anime' : isReadingMedia(e.media_type) ? 'manga' : 'other';

	async function load(): Promise<void> {
		loadError = '';
		try {
			const [res, linked] = await Promise.all([
				api.list.entries(),
				api.links.list().then((r) => r.links).catch(() => [] as LinkedAccountView[])
			]);
			entries = res.entries;
			links = linked;
			// Open on the media the list has, anime first.
			if (!entries.some((e) => mediaOf(e) === media)) {
				media = (['anime', 'manga', 'other'] as Media[]).find((m) => entries.some((e) => mediaOf(e) === m)) ?? 'anime';
			}
		} catch (err) {
			loadError = errorMessage(err, 'Could not load your list.');
		} finally {
			ready = true;
		}
	}

	onMount(() => void load());

	const link = $derived(links[0]);
	const firstRun = $derived(ready && !loadError && entries.length === 0);
	const reading = $derived(media === 'manga');

	const mediaCounts = $derived({
		anime: entries.filter((e) => mediaOf(e) === 'anime').length,
		manga: entries.filter((e) => mediaOf(e) === 'manga').length,
		other: entries.filter((e) => mediaOf(e) === 'other').length
	});
	const mediaOptions = $derived(
		(['anime', 'manga', 'other'] as Media[])
			.filter((m) => m !== 'other' || mediaCounts.other > 0)
			.map((m) => ({
				value: m,
				label: t('tracking.statusCount', {
					label: t(m === 'anime' ? 'tracking.media.anime' : m === 'manga' ? 'tracking.media.manga' : 'tracking.media.rest'),
					count: mediaCounts[m]
				})
			}))
	);
	const ofMedia = $derived(entries.filter((e) => mediaOf(e) === media && (!libraryOnly || !!e.item_id)));
	const lists = $derived([
		{ id: '', label: t('tracking.status.all'), count: ofMedia.length },
		...STATUSES.map((s) => ({ id: s, label: statusLabel(s, reading), count: ofMedia.filter((e) => e.status === s).length }))
	]);
	const shown = $derived(status ? ofMedia.filter((e) => e.status === status) : ofMedia);
	// What the library holds comes first (it opens), then by name.
	const order = (e: ListEntry) => (e.item_id ? 0 : 1);
	const sorted = $derived([...shown].sort((a, b) => order(a) - order(b) || a.title.localeCompare(b.title)));
	const inLibraryCount = $derived(entries.filter((e) => mediaOf(e) === media && !!e.item_id).length);

	function pickMedia(m: Media): void {
		media = m;
		status = '';
	}

	function stepList(by: number): void {
		const at = lists.findIndex((l) => l.id === status);
		status = lists[(at + by + lists.length) % lists.length].id;
	}

	async function sync(): Promise<void> {
		if (!link || syncing) return;
		syncing = true;
		try {
			const res = await api.links.sync(link.platform);
			toasts.success(t('tracking.linkedEmpty.synced', { count: res.stats.upserted }));
			await load();
		} catch (err) {
			toasts.error(errorMessage(err, t('tracking.linkedEmpty.syncFailed')));
		} finally {
			syncing = false;
		}
	}

	async function toggleAuto(): Promise<void> {
		if (!link || switching) return;
		switching = true;
		try {
			const next = await api.links.setScrobble(link.platform, !link.scrobble);
			links = links.map((l) => (l.platform === next.platform ? next : l));
			toasts.success(t(next.scrobble ? 'tracking.account.autoSaved' : 'tracking.account.autoOff'));
		} catch (err) {
			toasts.error(errorMessage(err, t('tracking.account.autoFailed')));
		} finally {
			switching = false;
		}
	}

	const lastPush = $derived.by(() => {
		const p = link?.last_push;
		if (!p) return '';
		const when = formatRelative(p.at);
		if (p.error) return t('tracking.account.lastPushFailed', { error: p.error, title: p.title, when });
		if (p.progress) return t('tracking.account.lastPush', { title: p.title, progress: p.progress, when });
		return t('tracking.account.lastPushStatus', { title: p.title, status: statusLabel(p.status ?? '', false), when });
	});

	usePageKeys(() => {
		if (firstRun) return [link ? { keys: ['S'], label: t('pageKeys.sync') } : { keys: ['C'], label: t('pageKeys.connect') }];
		if (!ready || loadError) return [];
		return [
			...(link ? [{ keys: ['S'], label: t('pageKeys.sync') }, { keys: ['A'], label: t('pageKeys.trackingAuto') }] : []),
			...(mediaOptions.length > 1 ? [{ keys: ['M'], label: t('pageKeys.trackingMedia') }] : []),
			{ keys: ['[', ']'], label: t('pageKeys.trackingStatus') },
			{ keys: ['I'], label: t('pageKeys.trackingLibrary') }
		];
	});

	function onKey(e: KeyboardEvent): void {
		if (!ready || loadError || e.ctrlKey || e.metaKey || e.altKey || e.defaultPrevented) return;
		if (isTypingTarget(e.target) || document.querySelector('[role="dialog"]')) return;
		const key = e.key.toLowerCase();
		if (firstRun) {
			if (key === 'c' && !link) {
				e.preventDefault();
				void goto('/settings/connections');
			} else if (key === 's' && link) {
				e.preventDefault();
				void sync();
			}
			return;
		}
		if (key === 's' && link) void sync();
		else if (key === 'a' && link) void toggleAuto();
		else if (key === 'm' && mediaOptions.length > 1) {
			const at = mediaOptions.findIndex((o) => o.value === media);
			pickMedia(mediaOptions[(at + 1) % mediaOptions.length].value);
		} else if (e.key === ']') stepList(1);
		else if (e.key === '[') stepList(-1);
		else if (key === 'i') libraryOnly = !libraryOnly;
		else return;
		e.preventDefault();
	}
</script>

<svelte:head><title>{t('tracking.pageTitle')}</title></svelte:head>
<svelte:window onkeydown={onKey} />

<div class="space-y-6">
	<header>
		<h1 class="page-title">{t('nav.list')}</h1>
		<!-- The first-run block below says what the page is for, at length. -->
		{#if !firstRun}<p class="mt-0.5 text-sm text-muted">{t('tracking.subtitle')}</p>{/if}
	</header>

	{#if link && !firstRun}
		<!-- Who is linked, when the list last came in, what lain last sent, and the switch that sends. -->
		<section class="flex flex-col gap-3 border-y border-hairline py-3 md:flex-row md:items-center md:justify-between" aria-label={platformName(link.platform)}>
			<div class="min-w-0 space-y-1">
				<p class="font-mono text-[11px] uppercase tracking-[0.16em] text-foreground/80">
					{t('tracking.account.label', { platform: platformName(link.platform), name: link.remote_username || link.remote_user_id })}
					<span class="text-muted">
						· {link.last_sync_error
							? t('tracking.account.syncFailed', { error: link.last_sync_error })
							: link.last_sync_at
								? t('tracking.account.syncedWhen', { when: formatRelative(link.last_sync_at) })
								: t('tracking.account.neverSynced')}
					</span>
				</p>
				<p class={['truncate text-xs', link.last_push?.error ? 'text-danger' : 'text-muted'].join(' ')} data-last-push>
					{lastPush || t('tracking.account.noPush')}
				</p>
			</div>
			<div class="flex flex-wrap items-center gap-3">
				<label class="flex items-center gap-2 text-sm text-foreground" title={link.scrobble ? t('tracking.account.autoOnHint') : t('tracking.account.autoOffHint', { platform: platformName(link.platform) })}>
					<Switch
						checked={link.scrobble}
						disabled={switching}
						label={t('tracking.account.auto', { platform: platformName(link.platform) })}
						bare
						onCheckedChange={() => void toggleAuto()}
					/>
					<span>{t('tracking.account.auto', { platform: platformName(link.platform) })}</span>
					<kbd>A</kbd>
				</label>
				<Button variant="ghost" size="field" loading={syncing} onclick={() => void sync()}>
					<RefreshCw class="size-3.5" /> {t('tracking.account.sync')} <kbd>S</kbd>
				</Button>
			</div>
		</section>
	{/if}

	{#if ready && !loadError && !firstRun}
		<div class="flex flex-wrap items-center gap-x-4 gap-y-3">
			{#if mediaOptions.length > 1}
				<Segmented options={mediaOptions} value={media} label={t('tracking.media.label')} size="sm" onchange={pickMedia} />
			{/if}
			<div class="flex flex-wrap items-center gap-2" role="group" aria-label={t('tracking.statusFilter')}>
				{#each lists as l (l.id)}
					<button
						type="button"
						aria-pressed={status === l.id}
						class={[
							'inline-flex min-h-9 items-center gap-1.5 rounded-full border px-3.5 text-sm font-medium transition-colors',
							status === l.id
								? 'border-accent bg-accent/15 text-foreground'
								: 'border-line bg-surface/60 text-muted hover:text-foreground'
						].join(' ')}
						onclick={() => (status = l.id)}
					>
						{l.label}<span class="font-mono text-[11px] tabular-nums opacity-70">{l.count}</span>
					</button>
				{/each}
			</div>
			<button
				type="button"
				aria-pressed={libraryOnly}
				class={[
					'inline-flex min-h-9 items-center gap-1.5 rounded-full border px-3.5 text-sm font-medium transition-colors',
					libraryOnly ? 'border-accent bg-accent/15 text-foreground' : 'border-line bg-surface/60 text-muted hover:text-foreground'
				].join(' ')}
				onclick={() => (libraryOnly = !libraryOnly)}
			>
				{t('tracking.inLibraryOnly')}<span class="font-mono text-[11px] tabular-nums opacity-70">{inLibraryCount}</span><kbd>I</kbd>
			</button>
		</div>
	{/if}

	{#if loadError}
		<ErrorState message={loadError} retry={() => void load()} />
	{:else if !ready}
		<div class={framedGrid('portrait')}>
			{#each Array(12) as _}
				<Skeleton class="aspect-[2/3] w-full rounded-xl" />
			{/each}
		</div>
	{:else if entries.length === 0 && link}
		<FirstRun
			art="list"
			eyebrow={t('tracking.linkedEmpty.eyebrow')}
			title={t('tracking.linkedEmpty.title')}
			body={t('tracking.linkedEmpty.body', { name: link.remote_username || link.platform })}
		>
			<Button size="rowMain" loading={syncing} onclick={() => void sync()}>{t('tracking.linkedEmpty.sync')} <kbd>S</kbd></Button>
			<LinkButton href="/settings/connections" variant="tonal" size="row">{t('tracking.linkedEmpty.connections')}</LinkButton>
		</FirstRun>
	{:else if entries.length === 0}
		<FirstRun
			art="list"
			eyebrow={t('tracking.firstRun.eyebrow')}
			title={t('tracking.firstRun.title')}
			body={t('tracking.firstRun.body')}
			steps={[
				{ label: t('tracking.firstRun.stepConnect'), text: t('tracking.firstRun.stepConnectText') },
				{ label: t('tracking.firstRun.stepImport'), text: t('tracking.firstRun.stepImportText') },
				{ label: t('tracking.firstRun.stepWatch'), text: t('tracking.firstRun.stepWatchText') }
			]}
		>
			<LinkButton href="/settings/connections" size="rowMain">{t('tracking.firstRun.connect')} <kbd>C</kbd></LinkButton>
			<LinkButton href="/library" variant="tonal" size="row">{t('tracking.firstRun.watch')}</LinkButton>
		</FirstRun>
	{:else if sorted.length === 0}
		<EmptyState
			title={libraryOnly ? t('tracking.emptyLibrary') : t('tracking.emptyStatus', { platform: platformName(link?.platform ?? 'anilist') })}
			description={t('tracking.noMatchBody')}
		>
			{#snippet icon()}<ListChecks class="size-6 text-muted" />{/snippet}
			<button
				type="button"
				class="inline-flex h-10 items-center rounded-md border border-line px-4 text-sm font-medium text-foreground hover:bg-surface-hover"
				onclick={() => {
					status = '';
					libraryOnly = false;
				}}>{t('tracking.showEverything')}</button
			>
		</EmptyState>
	{:else}
		<div class={framedGrid('portrait')}>
			{#each sorted as entry, i (entry.id)}
				<ListEntryCard {entry} priority={i < 6} canAcquire={session.isAdmin} />
			{/each}
		</div>
	{/if}
</div>
