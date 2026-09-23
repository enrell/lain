<script lang="ts">
	import { onMount } from 'svelte';
	import { api } from '$lib/api';
	import type { Library } from '$lib/api/types';
	import { session } from '$lib/auth/session.svelte';
	import { toasts } from '$lib/stores/toasts.svelte';
	import {
		EFFECT_PRESETS, defaultEffectsPolicy, loadEffectsPolicy, saveEffectsPolicy,
		type EffectPreset, type EffectRule, type EffectsPolicy
	} from '$lib/player/effects-policy';

	let policy = $state<EffectsPolicy>(defaultEffectsPolicy());
	let libraries = $state<Library[]>([]);
	let error = $state<string | null>(null);
	let loaded = $state(false);

	onMount(() => {
		if (session.user) policy = loadEffectsPolicy(session.user.id);
		loaded = true;
		void api.libraries.list().then((value) => { libraries = value; }).catch(() => {
			error = 'Could not load libraries. Existing rules can still be edited.';
		});
	});

	function save(): void {
		if (!session.user || !saveEffectsPolicy(session.user.id, policy)) {
			error = 'Could not save video effect preferences in this browser.';
			return;
		}
		error = null;
		toasts.success('Video effect preferences saved in this browser.');
	}

	function addRule(): void {
		policy = { ...policy, rules: [...policy.rules, { libraryType: 'anime', preset: 'off' }] };
	}

	function removeRule(index: number): void {
		policy = { ...policy, rules: policy.rules.filter((_, i) => i !== index) };
	}

	function updateRule(index: number, patch: Partial<EffectRule>): void {
		policy = { ...policy, rules: policy.rules.map((rule, i) => i === index ? { ...rule, ...patch } : rule) };
	}

	function nonnegative(value: string): number | undefined {
		if (value.trim() === '') return undefined;
		const number = Number(value);
		return Number.isInteger(number) && number >= 0 ? number : undefined;
	}
</script>

<svelte:head><title>Video effects — Settings — Lain</title></svelte:head>

<div class="space-y-6">
	<section class="rounded-card border border-line bg-surface/60 p-5">
		<h2 class="text-base font-semibold text-foreground">Video effects</h2>
		<p class="mt-1 text-sm text-muted">Anime4K runs in this browser with WebGPU or WebGL2. These preferences are saved only in this browser, for your account. The player can override a choice for the current viewing session.</p>
		{#if error}<p class="mt-3 text-sm text-danger" role="alert">{error}</p>{/if}
		<label class="mt-5 block text-sm font-medium text-foreground" for="effect-default">Default effect</label>
		<select id="effect-default" class="mt-2 w-full rounded-md border border-line bg-background p-2 text-foreground sm:max-w-md" bind:value={policy.default}>
			{#each EFFECT_PRESETS as preset}<option value={preset.id}>{preset.label}</option>{/each}
		</select>
		<p class="mt-2 text-xs text-muted">Mode A follows mpv Ctrl+1, A+A follows Ctrl+2. Lite restores detail at the source resolution.</p>
	</section>

	<section class="rounded-card border border-line bg-surface/60 p-5">
		<div class="flex flex-wrap items-center justify-between gap-3">
			<div><h2 class="text-base font-semibold text-foreground">Overrides</h2><p class="mt-1 text-sm text-muted">Set an effect for anime libraries, one library, a video track, or a source resolution range.</p></div>
			<button type="button" class="rounded-md border border-line px-3 py-2 text-sm text-foreground hover:bg-surface-hover" onclick={addRule}>Add override</button>
		</div>
		<p class="mt-3 text-xs text-muted">Priority: video track → resolution → library → library type → default. Later rows win ties. Height refers to the original video, before any upscale.</p>
		{#each policy.rules as rule, index (index)}
			<div class="mt-4 grid gap-3 rounded-md border border-line bg-background/70 p-4 sm:grid-cols-2 xl:grid-cols-3">
				<label class="text-xs text-muted">Effect
					<select class="mt-1 block w-full rounded-md border border-line bg-background p-2 text-sm text-foreground" value={rule.preset} onchange={(event) => updateRule(index, { preset: event.currentTarget.value as EffectPreset })}>
						{#each EFFECT_PRESETS as preset}<option value={preset.id}>{preset.label}</option>{/each}
					</select>
				</label>
				<label class="text-xs text-muted">Library type
					<select class="mt-1 block w-full rounded-md border border-line bg-background p-2 text-sm text-foreground" value={rule.libraryType ?? ''} onchange={(event) => updateRule(index, { libraryType: event.currentTarget.value || undefined })}>
						<option value="">Any type</option><option value="anime">Anime</option><option value="movie">Movie</option><option value="series">Series</option><option value="music">Music</option>
					</select>
				</label>
				<label class="text-xs text-muted">Library
					<select class="mt-1 block w-full rounded-md border border-line bg-background p-2 text-sm text-foreground" value={rule.libraryId ?? ''} onchange={(event) => updateRule(index, { libraryId: event.currentTarget.value || undefined })}>
						<option value="">Any library</option>{#each libraries as library}<option value={library.id}>{library.name}</option>{/each}
					</select>
				</label>
				<label class="text-xs text-muted">Minimum source height (px)
					<input type="number" min="0" step="1" class="mt-1 block w-full rounded-md border border-line bg-background p-2 text-sm text-foreground" value={rule.minHeight ?? ''} onchange={(event) => updateRule(index, { minHeight: nonnegative(event.currentTarget.value) })} />
				</label>
				<label class="text-xs text-muted">Maximum source height (px)
					<input type="number" min="0" step="1" class="mt-1 block w-full rounded-md border border-line bg-background p-2 text-sm text-foreground" value={rule.maxHeight ?? ''} onchange={(event) => updateRule(index, { maxHeight: nonnegative(event.currentTarget.value) })} />
				</label>
				<label class="text-xs text-muted">Video track index
					<input type="number" min="0" step="1" class="mt-1 block w-full rounded-md border border-line bg-background p-2 text-sm text-foreground" value={rule.streamIndex ?? ''} onchange={(event) => updateRule(index, { streamIndex: nonnegative(event.currentTarget.value) })} />
				</label>
				<button type="button" class="justify-self-start text-sm text-danger hover:underline" onclick={() => removeRule(index)}>Remove override {index + 1}</button>
			</div>
		{/each}
	</section>
	<button type="button" disabled={!loaded} class="rounded-md bg-accent px-4 py-2 text-sm font-semibold text-background disabled:opacity-50" onclick={save}>Save video effects</button>
</div>
