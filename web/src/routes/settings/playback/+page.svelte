<script lang="ts">
	import { onMount } from 'svelte';
	import Plus from '@lucide/svelte/icons/plus';
	import X from '@lucide/svelte/icons/x';
	import { api } from '$lib/api';
	import type { Library } from '$lib/api/types';
	import { session } from '$lib/auth/session.svelte';
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
	import { SavedFlash } from '$lib/settings/flash.svelte';
	import { toasts } from '$lib/stores/toasts.svelte';
	import { errorMessage } from '$lib/utilities/errors';

	// Everything here applies on change: personal preferences never wait
	// for a Save button (the YOU save model).
	const flash = new SavedFlash();

	let player = $state<PreferredPlayer>('browser');
	let language = $state('');
	let languageError = $state('');
	let policy = $state<EffectsPolicy>(defaultEffectsPolicy());
	let libraries = $state<Library[]>([]);

	onMount(() => {
		if (!session.user) return;
		player = loadPreferredPlayer(session.user.id);
		language = session.user.preferred_language ?? '';
		policy = loadEffectsPolicy(session.user.id);
		void api.libraries.list().then((v) => (libraries = v)).catch(() => {});
	});

	function changePlayer(value: string): void {
		if (!session.user || (value !== 'browser' && value !== 'mpv' && value !== 'vlc')) return;
		player = value;
		savePreferredPlayer(session.user.id, value);
		flash.mark('player');
	}

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

	const field = 'h-9 rounded-md border px-2.5 text-sm text-foreground outline-none';
</script>

<svelte:head><title>Playback — Settings — Lain</title></svelte:head>

<SettingsGroup title="Playing">
	<SettingRow id="player" label="Default player" hint="What Play opens from this browser." saved={flash.is('player')}>
		<select class="{field} w-44" value={player} onchange={(e) => changePlayer(e.currentTarget.value)} aria-label="Default player">
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
		label="Audio & subtitle language"
		hint="Audio in this language keeps subtitles off; otherwise subtitles in it. Follows your account."
		saved={flash.is('language')}
	>
		<input
			class="{field} w-28 font-mono uppercase"
			type="text"
			list="common-languages"
			maxlength="3"
			autocomplete="off"
			spellcheck="false"
			placeholder="auto"
			aria-label="Language code"
			aria-invalid={languageError ? 'true' : undefined}
			bind:value={language}
			onchange={() => void commitLanguage()}
			onkeydown={(e) => e.key === 'Enter' && void commitLanguage()}
		/>
		<datalist id="common-languages">
			<option value="por">Portuguese</option><option value="eng">English</option><option value="jpn">Japanese</option>
			<option value="spa">Spanish</option><option value="fra">French</option><option value="deu">German</option>
			<option value="ita">Italian</option><option value="kor">Korean</option><option value="zho">Chinese</option>
		</datalist>
		{#snippet below()}
			{#if languageError}<p class="text-xs text-danger" role="alert">{languageError}</p>{/if}
		{/snippet}
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
