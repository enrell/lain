<script lang="ts">
	/*
	 * Monitor a title, or edit a monitored one (docs/slices/acquisition.md,
	 * A-17/A-18/A-24). Opens on the title field; Enter saves.
	 */
	import { api, type Library, type Monitored, type Numbering, type Profile } from '$lib/api';
	import Button from '$lib/components/primitives/Button.svelte';
	import Modal from '$lib/components/primitives/Modal.svelte';
	import { formatSeasonMap, formatSeasonWants, parseSeasonMap, parseSeasonWants } from '$lib/acquire/format';
	import { t } from '$lib/i18n';
	import { toasts } from '$lib/stores/toasts.svelte';
	import { errorMessage } from '$lib/utilities/errors';

	let {
		open = $bindable(false),
		libraries,
		profiles,
		editing = null,
		prefill = '',
		onsaved
	}: {
		open?: boolean;
		libraries: Library[];
		profiles: Profile[];
		editing?: Monitored | null;
		prefill?: string;
		onsaved: (m: Monitored) => void;
	} = $props();

	const NUMBERINGS: Record<string, Numbering[]> = {
		anime: ['absolute', 'seasonal'],
		series: ['seasonal'],
		movie: ['movie'],
		manga: ['chapter', 'volume'],
		comic: ['chapter', 'volume']
	};

	let libraryId = $state('');
	let title = $state('');
	let aliases = $state('');
	let year = $state('');
	let profileId = $state('default');
	let numbering = $state<Numbering>('absolute');
	let from = $state('1');
	let to = $state('');
	let seasons = $state('');
	let seasonMap = $state('');
	let saving = $state(false);
	let titleInput = $state<HTMLInputElement>();

	const library = $derived(libraries.find((l) => l.id === libraryId));
	const allowed = $derived(NUMBERINGS[library?.type ?? 'anime'] ?? ['absolute']);

	$effect(() => {
		if (!open) return;
		const e = editing;
		libraryId = e?.library_id ?? libraries[0]?.id ?? '';
		title = e?.title ?? prefill;
		aliases = (e?.aliases ?? []).join(', ');
		year = e?.year ? String(e.year) : '';
		profileId = e?.profile_id ?? 'default';
		numbering = e?.numbering ?? (NUMBERINGS[libraries.find((l) => l.id === libraryId)?.type ?? 'anime'] ?? ['absolute'])[0];
		from = e?.from ? String(e.from) : '1';
		to = e?.to ? String(e.to) : '';
		seasons = formatSeasonWants(e?.seasons) || '1';
		seasonMap = formatSeasonMap(e?.season_map);
		requestAnimationFrame(() => requestAnimationFrame(() => titleInput?.focus()));
	});

	$effect(() => {
		if (!allowed.includes(numbering)) numbering = allowed[0];
	});

	async function save(e: SubmitEvent): Promise<void> {
		e.preventDefault();
		const map = parseSeasonMap(seasonMap);
		const wants = numbering === 'seasonal' ? parseSeasonWants(seasons) : [];
		if (map === null || wants === null) {
			toasts.error(t('acquire.monitor.badSeasons'));
			return;
		}
		const body: Partial<Monitored> = {
			library_id: libraryId,
			kind: library?.type,
			title,
			aliases: aliases.split(',').map((a) => a.trim()).filter(Boolean),
			year: Number(year) || undefined,
			profile_id: profileId,
			numbering,
			season_map: map,
			from: Number(from) || undefined,
			to: Number(to) || undefined,
			seasons: wants,
			enabled: editing?.enabled ?? true
		};
		saving = true;
		try {
			const saved = editing ? await api.acquire.updateMonitored(editing.id, body) : await api.acquire.monitor(body);
			open = false;
			toasts.success(t('acquire.monitor.saved', { title: saved.title }));
			onsaved(saved);
		} catch (err) {
			toasts.error(errorMessage(err, t('acquire.monitor.failed')));
		} finally {
			saving = false;
		}
	}

	const field = 'h-10 w-full rounded-md border border-transparent bg-field px-3 text-sm text-foreground placeholder:text-muted/60 focus:border-accent/60 focus:outline-none';
	const numberingLabel = (n: Numbering) => t(`acquire.numbering.${n}`);
</script>

<Modal bind:open title={editing ? t('acquire.monitor.editTitle', { title: editing.title }) : t('acquire.monitor.title')} description={t('acquire.monitor.description')}>
	<form id="monitor-form" class="grid gap-3 sm:grid-cols-2" onsubmit={save}>
		<label class="block space-y-1.5 sm:col-span-2"><span class="text-xs text-muted">{t('acquire.monitor.titleField')}</span>
			<input bind:this={titleInput} bind:value={title} required maxlength="200" class={field} />
		</label>
		<label class="block space-y-1.5"><span class="text-xs text-muted">{t('acquire.search.library')}</span>
			<select bind:value={libraryId} class={field} disabled={!!editing}>
				{#each libraries as l (l.id)}<option value={l.id}>{l.name} · {l.type}</option>{/each}
			</select>
		</label>
		<label class="block space-y-1.5"><span class="text-xs text-muted">{t('acquire.monitor.profile')}</span>
			<select bind:value={profileId} class={field}>
				{#each profiles as p (p.id)}<option value={p.id}>{p.name}</option>{/each}
			</select>
		</label>
		<label class="block space-y-1.5"><span class="text-xs text-muted">{t('acquire.monitor.numbering')}</span>
			<select bind:value={numbering} class={field}>
				{#each allowed as n (n)}<option value={n}>{numberingLabel(n)}</option>{/each}
			</select>
		</label>
		<label class="block space-y-1.5"><span class="text-xs text-muted">{t('acquire.monitor.year')}</span>
			<input bind:value={year} type="number" min="0" class="{field} font-mono" />
		</label>
		{#if numbering === 'seasonal'}
			<label class="block space-y-1.5 sm:col-span-2"><span class="text-xs text-muted">{t('acquire.monitor.seasons')}</span>
				<input bind:value={seasons} spellcheck={false} class="{field} font-mono text-xs" placeholder="1:1-12, 2:1-" />
			</label>
		{:else if numbering !== 'movie'}
			<label class="block space-y-1.5"><span class="text-xs text-muted">{t('acquire.monitor.from')}</span>
				<input bind:value={from} type="number" min="1" class="{field} font-mono" />
			</label>
			<label class="block space-y-1.5"><span class="text-xs text-muted">{t('acquire.monitor.to')}</span>
				<input bind:value={to} type="number" min="0" class="{field} font-mono" placeholder={t('acquire.monitor.ongoing')} />
			</label>
		{/if}
		{#if library?.type === 'anime'}
			<label class="block space-y-1.5 sm:col-span-2"><span class="text-xs text-muted">{t('acquire.monitor.seasonMap')}</span>
				<input bind:value={seasonMap} spellcheck={false} class="{field} font-mono text-xs" placeholder="1:1, 2:13, 3:25" />
			</label>
		{/if}
		<label class="block space-y-1.5 sm:col-span-2"><span class="text-xs text-muted">{t('acquire.monitor.aliases')}</span>
			<input bind:value={aliases} class={field} />
		</label>
	</form>
	{#snippet footer()}
		<Button variant="ghost" onclick={() => (open = false)}>{t('common.cancel')} <kbd class="ms-1 font-mono text-[10px] text-muted">Esc</kbd></Button>
		<Button type="submit" form="monitor-form" loading={saving} disabled={!title.trim() || !libraryId}>{t('acquire.monitor.save')} <kbd class="ms-1 font-mono text-[10px] opacity-70">Enter</kbd></Button>
	{/snippet}
</Modal>
