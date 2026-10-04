<script lang="ts">
	import type { Rating } from '$lib/api';
	import { t } from '$lib/i18n';
	import Button from '$lib/components/primitives/Button.svelte';
	import Modal from '$lib/components/primitives/Modal.svelte';
	import { socialError } from '$lib/social/format';
	import { toasts } from '$lib/stores/toasts.svelte';
	import { focusSoon } from './focus';
	import Kbd from './Kbd.svelte';
	import { FIELD } from './styles';

	let {
		open = $bindable(false),
		title,
		mine,
		onSave,
		onRemove
	}: {
		open?: boolean;
		title: string;
		mine?: Rating | null;
		onSave: (body: { score: number; review: string; spoiler: boolean }) => Promise<void>;
		onRemove: () => Promise<void>;
	} = $props();

	let score = $state(0);
	let review = $state('');
	let spoiler = $state(false);
	let saving = $state(false);
	let scores = $state<HTMLDivElement>();

	$effect(() => {
		if (!open) return;
		score = mine?.score ?? 0;
		review = mine?.review ?? '';
		spoiler = mine?.spoiler ?? false;
		focusSoon(() => scores?.querySelector<HTMLElement>('[aria-pressed="true"]') ?? scores?.querySelector<HTMLElement>('button'));
	});

	async function save(): Promise<void> {
		if (saving || (score === 0 && !review.trim())) return;
		saving = true;
		try {
			await onSave({ score, review, spoiler });
			open = false;
		} catch (err) {
			toasts.error(socialError(err, t('social.rate.failed')));
		} finally {
			saving = false;
		}
	}

	async function remove(): Promise<void> {
		saving = true;
		try {
			await onRemove();
			open = false;
		} catch (err) {
			toasts.error(socialError(err, t('social.rate.failed')));
		} finally {
			saving = false;
		}
	}

	function onKey(e: KeyboardEvent): void {
		if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) {
			e.preventDefault();
			void save();
			return;
		}
		// Digits pick the score while focus is not in the review: 1–9, 0 = 10.
		const el = e.target as HTMLElement;
		if (el.tagName === 'TEXTAREA' || e.ctrlKey || e.metaKey || e.altKey) return;
		if (/^[0-9]$/.test(e.key)) {
			e.preventDefault();
			score = e.key === '0' ? 10 : Number(e.key);
		}
	}
</script>

<Modal bind:open title={t('social.rate.title', { title })} description={t('social.rate.description')}>
	<!-- svelte-ignore a11y_no_static_element_interactions -->
	<div onkeydown={onKey} class="space-y-4">
		<div bind:this={scores} class="grid grid-cols-10 gap-1" role="group" aria-label={t('social.rate.score')}>
			{#each Array.from({ length: 10 }, (_, i) => i + 1) as n (n)}
				<button
					type="button"
					aria-pressed={score === n}
					aria-label={t('social.score', { score: n })}
					onclick={() => (score = score === n ? 0 : n)}
					class="h-9 rounded-md border font-mono text-sm transition-colors {score >= n && score > 0
						? 'border-accent bg-accent/15 text-foreground'
						: 'border-hairline text-muted hover:text-foreground'}"
				>
					{n}
				</button>
			{/each}
		</div>
		<p class="font-mono text-[10px] text-muted">{t('social.rate.keys')}</p>
		<label class="block space-y-1.5">
			<span class="text-xs text-muted">{t('social.rate.review')}</span>
			<textarea bind:value={review} maxlength="2000" rows="5" class={FIELD} placeholder={t('social.rate.reviewPlaceholder')}></textarea>
		</label>
		<label class="flex items-center gap-2 text-sm text-muted">
			<input type="checkbox" bind:checked={spoiler} class="accent-[var(--color-accent)]" />
			{t('social.rate.spoiler')}
		</label>
	</div>
	{#snippet footer()}
		{#if mine}
			<Button variant="danger" disabled={saving} onclick={() => void remove()}>{t('social.rate.remove')}</Button>
		{/if}
		<Button variant="ghost" onclick={() => (open = false)}>{t('common.cancel')} <Kbd>Esc</Kbd></Button>
		<Button loading={saving} disabled={score === 0 && !review.trim()} onclick={() => void save()}>
			{t('social.rate.save')} <Kbd>Ctrl ↵</Kbd>
		</Button>
	{/snippet}
</Modal>
