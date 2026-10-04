<script lang="ts">
	/*
	 * Create or edit a quality profile (A-19). Resolutions keep their
	 * best-first order; the cutoff is one of the allowed ones.
	 */
	import { api, type Profile } from '$lib/api';
	import Button from '$lib/components/primitives/Button.svelte';
	import Modal from '$lib/components/primitives/Modal.svelte';
	import Switch from '$lib/components/primitives/Switch.svelte';
	import { t } from '$lib/i18n';
	import { toasts } from '$lib/stores/toasts.svelte';
	import { errorMessage } from '$lib/utilities/errors';

	let { open = $bindable(false), editing = null, onsaved }: { open?: boolean; editing?: Profile | null; onsaved: () => void } = $props();

	const RESOLUTIONS = ['2160p', '1080p', '720p', '576p', '540p', '480p'];
	const SOURCES = ['bluray', 'web', 'hdtv', 'dvd'];

	let name = $state('');
	let resolutions = $state<string[]>([]);
	let sources = $state<string[]>([]);
	let cutoff = $state('1080p');
	let preferred = $state('');
	let blockedGroups = $state('');
	let blockedWords = $state('');
	let minSeeders = $state(1);
	let minSize = $state(0);
	let maxSize = $state(0);
	let proper = $state(true);
	let subLangs = $state('');
	let subWithAudio = $state(false);
	let subHI = $state<Profile['subtitle_hi']>('include');
	let saving = $state(false);
	let nameInput = $state<HTMLInputElement>();

	$effect(() => {
		if (!open) return;
		const p = editing;
		name = p?.name ?? '';
		resolutions = [...(p?.resolutions ?? ['2160p', '1080p', '720p'])];
		sources = [...(p?.sources ?? [])];
		cutoff = p?.cutoff ?? '1080p';
		preferred = (p?.preferred_groups ?? []).join(', ');
		blockedGroups = (p?.blocked_groups ?? []).join(', ');
		blockedWords = (p?.blocked_words ?? []).join(', ');
		minSeeders = p?.min_seeders ?? 1;
		minSize = p?.min_size_mb ?? 0;
		maxSize = p?.max_size_mb ?? 0;
		proper = p?.prefer_proper ?? true;
		subLangs = (p?.subtitle_languages ?? []).join(', ');
		subWithAudio = p?.subtitle_even_with_audio ?? false;
		subHI = p?.subtitle_hi ?? 'include';
		requestAnimationFrame(() => requestAnimationFrame(() => nameInput?.focus()));
	});

	function toggle(list: string[], v: string, order: string[]): string[] {
		const next = list.includes(v) ? list.filter((x) => x !== v) : [...list, v];
		return order.filter((x) => next.includes(x)); // keep best-first order
	}

	const csv = (s: string) => s.split(',').map((x) => x.trim()).filter(Boolean);

	async function save(e: SubmitEvent): Promise<void> {
		e.preventDefault();
		saving = true;
		const body = {
			name, resolutions, sources, cutoff: resolutions.includes(cutoff) ? cutoff : (resolutions[0] ?? ''),
			preferred_groups: csv(preferred), blocked_groups: csv(blockedGroups), blocked_words: csv(blockedWords),
			min_seeders: Number(minSeeders) || 0, min_size_mb: Number(minSize) || 0, max_size_mb: Number(maxSize) || 0, prefer_proper: proper,
			subtitle_languages: csv(subLangs), subtitle_even_with_audio: subWithAudio, subtitle_hi: subHI
		};
		try {
			if (editing) await api.acquire.updateProfile(editing.id, body);
			else await api.acquire.createProfile(body);
			open = false;
			toasts.success(t('acquire.profiles.saved', { name }));
			onsaved();
		} catch (err) {
			toasts.error(errorMessage(err, t('acquire.profiles.failed')));
		} finally {
			saving = false;
		}
	}

	const field = 'h-10 w-full rounded-md border border-transparent bg-field px-3 text-sm text-foreground placeholder:text-muted/60 focus:border-accent/60 focus:outline-none';
	const chip = (on: boolean) => `rounded-md border px-2.5 py-1 font-mono text-[11px] ${on ? 'border-accent bg-accent/15 text-foreground' : 'border-hairline text-muted hover:text-foreground'}`;
</script>

<Modal bind:open title={editing ? t('acquire.profiles.editTitle', { name: editing.name }) : t('acquire.profiles.addTitle')} description={t('acquire.profiles.description')}>
	<form id="profile-form" class="space-y-3" onsubmit={save}>
		<label class="block space-y-1.5"><span class="text-xs text-muted">{t('acquire.profiles.name')}</span>
			<input bind:this={nameInput} bind:value={name} required maxlength="60" class={field} />
		</label>
		<fieldset class="space-y-1.5"><legend class="text-xs text-muted">{t('acquire.profiles.resolutions')}</legend>
			<div class="flex flex-wrap gap-1.5">
				{#each RESOLUTIONS as r (r)}
					<button type="button" aria-pressed={resolutions.includes(r)} class={chip(resolutions.includes(r))} onclick={() => (resolutions = toggle(resolutions, r, RESOLUTIONS))}>{r}</button>
				{/each}
			</div>
		</fieldset>
		<label class="block space-y-1.5"><span class="text-xs text-muted">{t('acquire.profiles.cutoff')}</span>
			<select bind:value={cutoff} class={field}>
				{#each resolutions as r (r)}<option value={r}>{r}</option>{/each}
			</select>
		</label>
		<fieldset class="space-y-1.5"><legend class="text-xs text-muted">{t('acquire.profiles.sources')}</legend>
			<div class="flex flex-wrap gap-1.5">
				{#each SOURCES as s (s)}
					<button type="button" aria-pressed={sources.includes(s)} class={chip(sources.includes(s))} onclick={() => (sources = toggle(sources, s, SOURCES))}>{s}</button>
				{/each}
			</div>
		</fieldset>
		<label class="block space-y-1.5"><span class="text-xs text-muted">{t('acquire.profiles.preferred')}</span>
			<input bind:value={preferred} class={field} placeholder="Fansub-A, Fansub-B" />
		</label>
		<label class="block space-y-1.5"><span class="text-xs text-muted">{t('acquire.profiles.blockedGroups')}</span>
			<input bind:value={blockedGroups} class={field} />
		</label>
		<label class="block space-y-1.5"><span class="text-xs text-muted">{t('acquire.profiles.blockedWords')}</span>
			<input bind:value={blockedWords} class={field} placeholder="cam, ts" />
		</label>
		<div class="grid grid-cols-3 gap-3">
			<label class="block space-y-1.5"><span class="text-xs text-muted">{t('acquire.profiles.minSeeders')}</span>
				<input bind:value={minSeeders} type="number" min="0" class="{field} font-mono" />
			</label>
			<label class="block space-y-1.5"><span class="text-xs text-muted">{t('acquire.profiles.minSize')}</span>
				<input bind:value={minSize} type="number" min="0" class="{field} font-mono" />
			</label>
			<label class="block space-y-1.5"><span class="text-xs text-muted">{t('acquire.profiles.maxSize')}</span>
				<input bind:value={maxSize} type="number" min="0" class="{field} font-mono" />
			</label>
		</div>
		<Switch label={t('acquire.profiles.proper')} bind:checked={proper} />
		<fieldset class="space-y-3 border-t border-hairline pt-3">
			<legend class="font-mono text-[11px] uppercase tracking-wider text-muted">{t('acquire.profiles.subtitles')}</legend>
			<label class="block space-y-1.5"><span class="text-xs text-muted">{t('acquire.profiles.subtitleLanguages')}</span>
				<input bind:value={subLangs} class="{field} font-mono" placeholder="en, pt-BR" />
				<span class="block text-[11px] text-muted">{t('acquire.profiles.subtitleLanguagesHint')}</span>
			</label>
			<Switch label={t('acquire.profiles.subtitleWithAudio')} bind:checked={subWithAudio} />
			<label class="block space-y-1.5"><span class="text-xs text-muted">{t('acquire.profiles.subtitleHI')}</span>
				<select bind:value={subHI} class={field}>
					<option value="include">{t('acquire.profiles.hiInclude')}</option>
					<option value="prefer">{t('acquire.profiles.hiPrefer')}</option>
					<option value="exclude">{t('acquire.profiles.hiExclude')}</option>
				</select>
			</label>
		</fieldset>
	</form>
	{#snippet footer()}
		<Button variant="ghost" onclick={() => (open = false)}>{t('common.cancel')} <kbd class="ms-1 font-mono text-[10px] text-muted">Esc</kbd></Button>
		<Button type="submit" form="profile-form" loading={saving} disabled={!name.trim() || resolutions.length === 0}>{t('acquire.profiles.save')} <kbd class="ms-1 font-mono text-[10px] opacity-70">Enter</kbd></Button>
	{/snippet}
</Modal>
