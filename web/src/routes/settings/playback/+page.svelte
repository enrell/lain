<script lang="ts">
	import { nameable } from '$lib/stores/media-cache.svelte';
	import { onMount } from 'svelte';
	import Plus from '@lucide/svelte/icons/plus';
	import X from '@lucide/svelte/icons/x';
	import { api } from '$lib/api';
	import type { Library } from '$lib/api/types';
	import { session } from '$lib/auth/session.svelte';
	import Button from '$lib/components/primitives/Button.svelte';
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
		type EffectRenderer,
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
	import Segmented from '$lib/components/primitives/Segmented.svelte';
	import { SavedFlash } from '$lib/settings/flash.svelte';
	import { trackPreferences, type SubtitleMode } from '$lib/player/track-language';
	import { toasts } from '$lib/stores/toasts.svelte';
	import { errorMessage } from '$lib/utilities/errors';
	import { t } from '$lib/i18n';

	// Everything here applies on change: personal preferences never wait
	// for a Save button (the YOU save model).
	const flash = new SavedFlash();

	let player = $state<PreferredPlayer>('browser');
	// The account's track preferences (D-280): every client reads them.
	let audioLanguage = $state('');
	let subtitleLanguage = $state('');
	let subtitleMode = $state<SubtitleMode>('auto');
	let policy = $state<EffectsPolicy>(defaultEffectsPolicy());
	let libraries = $state<Library[]>([]);
	// The player's own choices, kept in this browser: the same keys its menu writes.
	let skip = $state(0);
	let subtitleSize = $state<SubtitleSize>('medium');
	let subtitleBox = $state(false);

	onMount(() => {
		if (!session.user) return;
		player = loadPreferredPlayer(session.user.id);
		({ audio: audioLanguage, subtitle: subtitleLanguage, mode: subtitleMode } = trackPreferences(session.user));
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
	// A code set elsewhere (the CLI, another client) that the list does not name stays choosable.
	function languageOptions(current: string, empty: string) {
		const extra: [string, string][] = current && !LANGUAGES.some(([c]) => c === current) ? [[current, current]] : [];
		return [['', empty] as [string, string], ...LANGUAGES, ...extra].map(([value, label]) => ({ value, label }));
	}
	const audioOptions = $derived(languageOptions(audioLanguage, t('trackPrefs.asFile')));
	const subtitleOptions = $derived(languageOptions(subtitleLanguage, t('trackPrefs.none')));
	const modeOptions = $derived([
		{ value: 'auto', label: t('trackPrefs.modeAuto') },
		{ value: 'always', label: t('trackPrefs.modeAlways') },
		{ value: 'forced', label: t('trackPrefs.modeForced') },
		{ value: 'off', label: t('trackPrefs.modeOff') }
	]);
	const modeHint = $derived(
		{
			auto: t('trackPrefs.hintAuto'),
			always: t('trackPrefs.hintAlways'),
			forced: t('trackPrefs.hintForced'),
			off: t('trackPrefs.hintOff')
		}[subtitleMode]
	);

	// Each row saves its own field on change; a refusal puts the row back.
	async function commitTracks(
		row: 'audio-language' | 'subtitle-language' | 'subtitle-mode',
		patch: Parameters<typeof api.me.updatePreferences>[0]
	): Promise<void> {
		try {
			session.user = await api.me.updatePreferences(patch);
			flash.mark(row);
		} catch (err) {
			toasts.error(errorMessage(err, t('trackPrefs.saveFailed')));
		}
		({ audio: audioLanguage, subtitle: subtitleLanguage, mode: subtitleMode } = trackPreferences(session.user));
	}
	const changeAudio = (v: string) => void commitTracks('audio-language', { preferred_audio_language: v });
	const changeSubtitle = (v: string) => void commitTracks('subtitle-language', { preferred_subtitle_language: v });
	function changeMode(v: string): void {
		if (v === 'auto' || v === 'always' || v === 'forced' || v === 'off') {
			void commitTracks('subtitle-mode', { preferred_subtitle_mode: v });
		}
	}

	function persistEffects(next: EffectsPolicy, row: string): void {
		policy = next;
		if (!session.user || !saveEffectsPolicy(session.user.id, next)) {
			toasts.error(t('settings.playbackPage.effectsFailed'));
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
	const field = 'h-10 rounded-md border px-3 text-sm text-foreground outline-none';
</script>

<svelte:head><title>{t('settings.playbackPage.pageTitle')}</title></svelte:head>

<SettingsGroup title={t('settings.playbackPage.playing')} description={t('settings.groups.playback.playing')}>
	<SettingRow id="player" label={t('settings.entry.player')} hint={t('settings.playbackPage.playerHint')} saved={flash.is('player')}>
		<select class="{field} w-full" value={player} onchange={(e) => changePlayer(e.currentTarget.value)} aria-label={t('settings.entry.player')}>
			<option value="browser">{t('settings.playbackPage.webPlayer')}</option>
			<option value="mpv">mpv</option>
			<option value="vlc">VLC</option>
		</select>
		{#snippet below()}
			{#if player !== 'browser'}
				<p class="font-mono text-[11px] text-muted">
					{t('settings.playbackPage.localHint')} <code class="text-foreground">lain login</code> ·
					<code class="text-foreground">lain install-player-handler</code>
				</p>
			{/if}
		{/snippet}
	</SettingRow>
	<SettingRow id="audio-language" label={t('trackPrefs.audio')} hint={t('trackPrefs.audioHint')} saved={flash.is('audio-language')}>
		<!-- A language by name, from a list: a three-letter code to type was a thing to know first. -->
		<!-- As wide as the native pickers on this page; the whole column on a phone, as they are. -->
		<div class="w-full">
			<Select aria-label={t('trackPrefs.audio')} bind:value={audioLanguage} options={audioOptions} onValueChange={changeAudio} />
		</div>
	</SettingRow>
	<SettingRow
		id="subtitle-language"
		label={t('trackPrefs.subtitle')}
		hint={t('trackPrefs.subtitleHint')}
		saved={flash.is('subtitle-language')}
	>
		<div class="w-full">
			<Select
				aria-label={t('trackPrefs.subtitle')}
				bind:value={subtitleLanguage}
				options={subtitleOptions}
				onValueChange={changeSubtitle}
			/>
		</div>
	</SettingRow>
	<SettingRow id="subtitle-mode" label={t('trackPrefs.mode')} hint={modeHint} saved={flash.is('subtitle-mode')}>
		<div class="w-full">
			<Select aria-label={t('trackPrefs.mode')} bind:value={subtitleMode} options={modeOptions} onValueChange={changeMode} />
		</div>
	</SettingRow>
</SettingsGroup>

<!-- What the app keeps in its Settings too; on the web they were only in the player's menu,
     and the subtitle ones only while a file with subtitles played. -->
<SettingsGroup id="player-prefs" title={t('playerPrefs.group')} description={t('settings.groups.playback.prefs')}>
	<SettingRow
		id="skip"
		label={t('playerPrefs.skip')}
		hint={skip === 0 ? t('playerPrefs.skipAutoHint') : t('playerPrefs.skipFixedHint', { seconds: skip })}
		saved={flash.is('skip')}
	>
		<select class="{field} w-full" value={String(skip)} onchange={(e) => changeSkip(e.currentTarget.value)} aria-label={t('playerPrefs.skip')}>
			{#each SKIP_CHOICES as option (option)}
				<option value={String(option)}>{option === 0 ? t('playerPrefs.skipAuto') : t('playerPrefs.skipSeconds', { seconds: option })}</option>
			{/each}
		</select>
	</SettingRow>
	<SettingRow id="subtitle-size" label={t('playerPrefs.subtitleSize')} saved={flash.is('subtitle-size')}>
		<select
			class="{field} w-full"
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

<SettingsGroup id="effects" title={t('settings.playbackPage.effects')} description={t('settings.groups.playback.effects')}>
	<SettingRow
		label={t('settings.playbackPage.defaultEffect')}
		hint={t('settings.playbackPage.defaultEffectHint')}
		saved={flash.is('effect-default')}
	>
		<select
			class="{field} w-full"
			aria-label={t('settings.playbackPage.defaultEffect')}
			value={policy.default}
			onchange={(e) => persistEffects({ ...policy, default: e.currentTarget.value as EffectPreset }, 'effect-default')}
		>
			{#each EFFECT_PRESETS as preset (preset.id)}<option value={preset.id}>{preset.label}</option>{/each}
		</select>
	</SettingRow>
	<SettingRow
		label={t('settings.playbackPage.renderOn')}
		hint={t('settings.playbackPage.renderOnHint')}
		saved={flash.is('effect-render')}
	>
		<!-- Two places, one choice: the app's segmented control, not a select
		     narrower than every other one on the page. -->
		<Segmented
			label={t('settings.playbackPage.renderOnLabel')}
			value={policy.renderOn ?? 'client'}
			options={[
				{ value: 'client', label: t('settings.playbackPage.thisDevice') },
				{ value: 'server', label: t('settings.playbackPage.server') }
			]}
			onchange={(v) => persistEffects({ ...policy, renderOn: v as EffectRenderer }, 'effect-render')}
		/>
	</SettingRow>
	<SettingRow
		label={t('settings.playbackPage.overrides')}
		hint={t('settings.playbackPage.overridesHint')}
		saved={flash.is('overrides')}
		stack
	>
		{#if policy.rules.length > 0}
			<div class="overflow-x-auto">
				<table class="w-full min-w-[46rem] text-sm">
					<thead>
						<tr class="text-left font-mono text-[10px] uppercase tracking-[0.14em] text-muted">
							<th class="pb-2 text-start font-normal">{t('settings.playbackPage.colEffect')}</th><th class="pb-2 text-start font-normal">{t('settings.playbackPage.colType')}</th><th class="pb-2 text-start font-normal">{t('settings.playbackPage.colLibrary')}</th>
							<th class="pb-2 text-start font-normal">{t('settings.playbackPage.colMin')}</th><th class="pb-2 text-start font-normal">{t('settings.playbackPage.colMax')}</th><th class="pb-2 text-start font-normal">{t('settings.playbackPage.colTrack')}</th><th><span class="sr-only">{t('settings.transcode.encoding.colRemove')}</span></th>
						</tr>
					</thead>
					<tbody>
						{#each policy.rules as rule, index (index)}
							<tr class="border-t border-hairline">
								<td class="py-1.5 pr-2">
									<select class="{field} w-full" aria-label={t('settings.playbackPage.ruleEffect', { n: index + 1 })} value={rule.preset} onchange={(e) => updateRule(index, { preset: e.currentTarget.value as EffectPreset })}>
										{#each EFFECT_PRESETS as preset (preset.id)}<option value={preset.id}>{preset.label}</option>{/each}
									</select>
								</td>
								<td class="py-1.5 pr-2">
									<select class="{field} w-full" aria-label={t('settings.playbackPage.ruleType', { n: index + 1 })} value={rule.libraryType ?? ''} onchange={(e) => updateRule(index, { libraryType: e.currentTarget.value || undefined })}>
										<option value="">{t('settings.playbackPage.any')}</option><option value="anime">{t('settings.librariesPage.types.anime')}</option><option value="movie">{t('settings.librariesPage.types.movie')}</option><option value="series">{t('settings.librariesPage.types.series')}</option>
									</select>
								</td>
								<td class="py-1.5 pr-2">
									<select class="{field} w-full" aria-label={t('settings.playbackPage.ruleLibrary', { n: index + 1 })} value={rule.libraryId ?? ''} onchange={(e) => updateRule(index, { libraryId: e.currentTarget.value || undefined })}>
										<option value="">{t('settings.playbackPage.any')}</option>{#each libraries as library (library.id)}<option value={library.id}>{library.name}</option>{/each}
									</select>
								</td>
								<td class="py-1.5 pr-2"><input type="number" min="0" class="{field} w-20 font-mono" aria-label={t('settings.playbackPage.minHeight')} value={rule.minHeight ?? ''} onchange={(e) => updateRule(index, { minHeight: nonnegative(e.currentTarget.value) })} /></td>
								<td class="py-1.5 pr-2"><input type="number" min="0" class="{field} w-20 font-mono" aria-label={t('settings.playbackPage.maxHeight')} value={rule.maxHeight ?? ''} onchange={(e) => updateRule(index, { maxHeight: nonnegative(e.currentTarget.value) })} /></td>
								<td class="py-1.5 pr-2"><input type="number" min="0" class="{field} w-16 font-mono" aria-label={t('settings.playbackPage.track')} value={rule.streamIndex ?? ''} onchange={(e) => updateRule(index, { streamIndex: nonnegative(e.currentTarget.value) })} /></td>
								<td class="py-1.5 text-right">
									<button type="button" class="inline-flex size-8 items-center justify-center rounded-md text-muted hover:bg-surface-hover hover:text-danger" aria-label={t('settings.playbackPage.ruleRemove', { n: index + 1 })} onclick={() => removeRule(index)}>
										<X class="size-4" />
									</button>
								</td>
							</tr>
						{/each}
					</tbody>
				</table>
			</div>
		{/if}
		<Button variant="tonal" size="field" onclick={addRule}>
			<Plus class="size-4" /> {t('settings.playbackPage.addOverride')}
		</Button>
	</SettingRow>
</SettingsGroup>
