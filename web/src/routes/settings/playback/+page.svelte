<script lang="ts">
	import { nameable } from '$lib/stores/media-cache.svelte';
	import { onMount } from 'svelte';
	import Plus from '@lucide/svelte/icons/plus';
	import X from '@lucide/svelte/icons/x';
	import { api } from '$lib/api';
	import type { Library } from '$lib/api/types';
	import { session } from '$lib/auth/session.svelte';
	import Select from '$lib/components/primitives/Select.svelte';
	import SettingRow from '$lib/components/settings/SettingRow.svelte';
	import SettingsGroup from '$lib/components/settings/SettingsGroup.svelte';
	import { loadPreferredPlayer, savePreferredPlayer, type PreferredPlayer } from '$lib/player/external-player';
	import {
		EFFECT_PRESETS,
		defaultEffectsPolicy,
		loadEffectsPolicy,
		saveEffectsPolicy,
		type EffectPreset,
		type EffectRule,
		type EffectsPolicy
	} from '$lib/player/effects-policy';
	import {
		SKIP_CHOICES,
		loadSkip,
		loadSubtitleBackground,
		loadSubtitleSize,
		saveSkip,
		saveSubtitleBackground,
		saveSubtitleSize,
		type SubtitleSize
	} from '$lib/player/player-prefs';
	import Switch from '$lib/components/primitives/Switch.svelte';
	import { SavedFlash } from '$lib/settings/flash.svelte';
	import { toasts } from '$lib/stores/toasts.svelte';
	import { errorMessage } from '$lib/utilities/errors';
	import { t } from '$lib/i18n';

	// Everything here applies on change: personal preferences never wait
	// for a Save button (the YOU save model).
	const flash = new SavedFlash();

	let player = $state<PreferredPlayer>('browser');
	let language = $state('');
	let languageError = $state('');
	let policy = $state<EffectsPolicy>(defaultEffectsPolicy());
	let libraries = $state<Library[]>([]);
	// The player's own choices, kept in this browser: the same keys its menu writes.
	let skip = $state(0);
	let subtitleSize = $state<SubtitleSize>('medium');
	let subtitleBox = $state(false);

	onMount(() => {
		if (!session.user) return;
		player = loadPreferredPlayer(session.user.id);
		language = session.user.preferred_language ?? '';
		policy = loadEffectsPolicy(session.user.id);
		skip = loadSkip();
		subtitleSize = loadSubtitleSize();
		subtitleBox = loadSubtitleBackground() === 'box';
		void api.libraries.list().then((v) => (libraries = nameable(v))).catch(() => {});
	});

	function changePlayer(value: string): void {
		if (!session.user || (value !== 'browser' && value !== 'mpv' && value !== 'vlc')) return;
		player = value;
		savePreferredPlayer(session.user.id, value);
		flash.mark('player');
	}

	function changeSkip(value: string): void {
		const n = Number(value);
		if (!(SKIP_CHOICES as readonly number[]).includes(n)) return;
		skip = n;
		saveSkip(n);
		flash.mark('skip');
	}
	function changeSubtitleSize(value: string): void {
		if (value !== 'small' && value !== 'medium' && value !== 'large') return;
		subtitleSize = value;
		saveSubtitleSize(value);
		flash.mark('subtitle-size');
	}
	function changeSubtitleBox(on: boolean): void {
		subtitleBox = on;
		saveSubtitleBackground(on ? 'box' : 'none');
		flash.mark('subtitle-box');
	}

	const LANGUAGES: [string, string][] = [
		// Said as the app says it: one account setting, two names for it read as two settings.
		['', t('playbackLanguage.asFile')],
		['eng', 'English'],
		['por', 'Portuguese'],
		['spa', 'Spanish'],
		['fra', 'French'],
		['deu', 'German'],
		['ita', 'Italian'],
		['jpn', 'Japanese'],
		['kor', 'Korean'],
		['zho', 'Chinese']
	];
	// A code set elsewhere that the list does not name stays choosable.
	const languageOptions = $derived(
		[...LANGUAGES, ...(language && !LANGUAGES.some(([c]) => c === language) ? [[language, language] as [string, string]] : [])].map(
			([value, label]) => ({ value, label })
		)
	);

	async function commitLanguage(): Promise<void> {
		const next = language.trim().toLowerCase();
		if (next === (session.user?.preferred_language ?? '')) return;
		if (next && !/^[a-z]{3}$/.test(next)) {
			languageError = 'Three letters, ISO 639-2 (por, eng, jpn).';
			return;
		}
		languageError = '';
		try {
			session.user = await api.me.setPreferredLanguage(next);
			language = next;
			flash.mark('language');
		} catch (err) {
			toasts.error(errorMessage(err, 'Could not save the language.'));
		}
	}

	function persistEffects(next: EffectsPolicy, row: string): void {
		policy = next;
		if (!session.user || !saveEffectsPolicy(session.user.id, next)) {
			toasts.error('Could not save video effects in this browser.');
			return;
		}
		flash.mark(row);
	}
	const addRule = () =>
		persistEffects({ ...policy, rules: [...policy.rules, { libraryType: 'anime', preset: 'off' }] }, 'overrides');
	const removeRule = (i: number) =>
		persistEffects({ ...policy, rules: policy.rules.filter((_, j) => j !== i) }, 'overrides');
	const updateRule = (i: number, patch: Partial<EffectRule>) =>
		persistEffects({ ...policy, rules: policy.rules.map((r, j) => (j === i ? { ...r, ...patch } : r)) }, 'overrides');
	function nonnegative(value: string): number | undefined {
		if (value.trim() === '') return undefined;
		const n = Number(value);
		return Number.isInteger(n) && n >= 0 ? n : undefined;
	}

	// The height of the Select trigger beside them (h-10): one row of pickers, one size.
	const field = 'h-10 rounded-md border px-2.5 text-sm text-foreground outline-none';
</script>

<svelte:head><title>Playback — Settings — Lain</title></svelte:head>

<SettingsGroup title="Playing">
	<SettingRow id="player" label="Default player" hint="Which player opens when you press Play in this browser." saved={flash.is('player')}>
		<select class="{field} w-56" value={player} onchange={(e) => changePlayer(e.currentTarget.value)} aria-label="Default player">
			<option value="browser">Web player</option>
			<option value="mpv">mpv</option>
			<option value="vlc">VLC</option>
		</select>
		{#snippet below()}
			{#if player !== 'browser'}
				<p class="font-mono text-[11px] text-muted">
					Same machine: opens directly. Elsewhere: <code class="text-foreground">lain login</code> ·
					<code class="text-foreground">lain install-player-handler</code>
				</p>
			{/if}
		{/snippet}
	</SettingRow>
	<SettingRow
		id="language"
		label={t('playbackLanguage.label')}
		hint={t('playbackLanguage.hint')}
		saved={flash.is('language')}
	>
		<!-- A language by name, from a list: a three-letter code to type was a thing to know first. -->
		<!-- As wide as the native pickers on this page; the whole column on a phone, as they are. -->
		<div class="w-full md:w-56">
			<Select
				aria-label="Audio and subtitle language"
				bind:value={language}
				options={languageOptions}
				onValueChange={() => void commitLanguage()}
			/>
		</div>
		{#snippet below()}
			{#if languageError}<p class="text-xs text-danger" role="alert">{languageError}</p>{/if}
		{/snippet}
	</SettingRow>
</SettingsGroup>

<!-- What the app keeps in its Settings too; on the web they were only in the player's menu,
     and the subtitle ones only while a file with subtitles played. -->
<SettingsGroup id="player-prefs" title={t('playerPrefs.group')}>
	<SettingRow
		id="skip"
		label={t('playerPrefs.skip')}
		hint={skip === 0 ? t('playerPrefs.skipAutoHint') : t('playerPrefs.skipFixedHint', { seconds: skip })}
		saved={flash.is('skip')}
	>
		<select class="{field} w-56" value={String(skip)} onchange={(e) => changeSkip(e.currentTarget.value)} aria-label={t('playerPrefs.skip')}>
			{#each SKIP_CHOICES as option (option)}
				<option value={String(option)}>{option === 0 ? t('playerPrefs.skipAuto') : t('playerPrefs.skipSeconds', { seconds: option })}</option>
			{/each}
		</select>
	</SettingRow>
	<SettingRow id="subtitle-size" label={t('playerPrefs.subtitleSize')} saved={flash.is('subtitle-size')}>
		<select
			class="{field} w-56"
			value={subtitleSize}
			onchange={(e) => changeSubtitleSize(e.currentTarget.value)}
			aria-label={t('playerPrefs.subtitleSize')}
		>
			<option value="small">{t('playerPrefs.sizeSmall')}</option>
			<option value="medium">{t('playerPrefs.sizeMedium')}</option>
			<option value="large">{t('playerPrefs.sizeLarge')}</option>
		</select>
	</SettingRow>
	<SettingRow id="subtitle-box" label={t('playerPrefs.subtitleBox')} hint={t('playerPrefs.subtitleBoxHint')} saved={flash.is('subtitle-box')}>
		<Switch bare label={t('playerPrefs.subtitleBox')} checked={subtitleBox} onCheckedChange={changeSubtitleBox} />
	</SettingRow>
</SettingsGroup>

<SettingsGroup id="effects" title="Video effects · this browser">
	<SettingRow
		label="Default effect"
		hint="Anime4K via WebGPU or WebGL2. The player can override it per session."
		saved={flash.is('effect-default')}
	>
		<select
			class="{field} w-56"
			aria-label="Default effect"
			value={policy.default}
			onchange={(e) => persistEffects({ ...policy, default: e.currentTarget.value as EffectPreset }, 'effect-default')}
		>
			{#each EFFECT_PRESETS as preset (preset.id)}<option value={preset.id}>{preset.label}</option>{/each}
		</select>
	</SettingRow>
	<SettingRow
		label="Overrides"
		hint="Track → resolution → library → type → default. Later rows win ties."
		saved={flash.is('overrides')}
		stack
	>
		{#if policy.rules.length > 0}
			<div class="overflow-x-auto">
				<table class="w-full min-w-[46rem] text-sm">
					<thead>
						<tr class="text-left font-mono text-[10px] uppercase tracking-[0.14em] text-muted">
							<th class="pb-2 font-normal">Effect</th><th class="pb-2 font-normal">Type</th><th class="pb-2 font-normal">Library</th>
							<th class="pb-2 font-normal">Min h</th><th class="pb-2 font-normal">Max h</th><th class="pb-2 font-normal">Track</th><th></th>
						</tr>
					</thead>
					<tbody>
						{#each policy.rules as rule, index (index)}
							<tr class="border-t border-hairline">
								<td class="py-1.5 pr-2">
									<select class="{field} w-full" aria-label="Effect for override {index + 1}" value={rule.preset} onchange={(e) => updateRule(index, { preset: e.currentTarget.value as EffectPreset })}>
										{#each EFFECT_PRESETS as preset (preset.id)}<option value={preset.id}>{preset.label}</option>{/each}
									</select>
								</td>
								<td class="py-1.5 pr-2">
									<select class="{field} w-full" aria-label="Library type for override {index + 1}" value={rule.libraryType ?? ''} onchange={(e) => updateRule(index, { libraryType: e.currentTarget.value || undefined })}>
										<option value="">any</option><option value="anime">anime</option><option value="movie">movie</option><option value="series">series</option>
									</select>
								</td>
								<td class="py-1.5 pr-2">
									<select class="{field} w-full" aria-label="Library for override {index + 1}" value={rule.libraryId ?? ''} onchange={(e) => updateRule(index, { libraryId: e.currentTarget.value || undefined })}>
										<option value="">any</option>{#each libraries as library (library.id)}<option value={library.id}>{library.name}</option>{/each}
									</select>
								</td>
								<td class="py-1.5 pr-2"><input type="number" min="0" class="{field} w-20 font-mono" aria-label="Minimum height" value={rule.minHeight ?? ''} onchange={(e) => updateRule(index, { minHeight: nonnegative(e.currentTarget.value) })} /></td>
								<td class="py-1.5 pr-2"><input type="number" min="0" class="{field} w-20 font-mono" aria-label="Maximum height" value={rule.maxHeight ?? ''} onchange={(e) => updateRule(index, { maxHeight: nonnegative(e.currentTarget.value) })} /></td>
								<td class="py-1.5 pr-2"><input type="number" min="0" class="{field} w-16 font-mono" aria-label="Video track" value={rule.streamIndex ?? ''} onchange={(e) => updateRule(index, { streamIndex: nonnegative(e.currentTarget.value) })} /></td>
								<td class="py-1.5 text-right">
									<button type="button" class="inline-flex size-8 items-center justify-center rounded-md text-muted hover:bg-surface-hover hover:text-danger" aria-label="Remove override {index + 1}" onclick={() => removeRule(index)}>
										<X class="size-4" />
									</button>
								</td>
							</tr>
						{/each}
					</tbody>
				</table>
			</div>
		{/if}
		<button type="button" class="inline-flex items-center gap-1.5 text-sm text-muted hover:text-foreground" onclick={addRule}>
			<Plus class="size-4" /> Add override
		</button>
	</SettingRow>
</SettingsGroup>
