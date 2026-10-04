<script lang="ts">
	import { api, type Collection, type WorkRef, targetOf } from '$lib/api';
	import { session } from '$lib/auth/session.svelte';
	import { t } from '$lib/i18n';
	import Button from '$lib/components/primitives/Button.svelte';
	import Modal from '$lib/components/primitives/Modal.svelte';
	import { socialError, workKey } from '$lib/social/format';
	import { toasts } from '$lib/stores/toasts.svelte';
	import { focusSoon } from './focus';
	import Kbd from './Kbd.svelte';
	import { FIELD } from './styles';

	let { open = $bindable(false), work }: { open?: boolean; work: WorkRef } = $props();

	let mine = $state<Collection[]>([]);
	let ready = $state(false);
	let busy = $state('');
	let name = $state('');
	let list = $state<HTMLDivElement>();
	let nameInput = $state<HTMLInputElement>();

	const key = $derived(workKey(work));
	const holds = (c: Collection) => c.items.some((it) => workKey(it.work) === key);

	$effect(() => {
		if (!open) return;
		ready = false;
		name = '';
		api.social
			.collections()
			.then((res) => {
				mine = res.collections.filter((c) => c.owner_id === session.user?.id);
				ready = true;
				focusSoon(() => list?.querySelector<HTMLElement>('button') ?? nameInput);
			})
			.catch((err) => {
				toasts.error(socialError(err, t('social.collect.loadFailed')));
				open = false;
			});
	});

	async function toggle(c: Collection): Promise<void> {
		busy = c.id;
		try {
			const res = holds(c)
				? await api.social.removeFromCollection(c.id, targetOf(work))
				: await api.social.addToCollection(c.id, targetOf(work));
			mine = mine.map((m) => (m.id === c.id ? res.collection : m));
		} catch (err) {
			toasts.error(socialError(err, t('social.collect.failed')));
		} finally {
			busy = '';
		}
	}

	async function create(e: SubmitEvent): Promise<void> {
		e.preventDefault();
		if (!name.trim()) return;
		busy = 'new';
		try {
			const res = await api.social.createCollection({ name });
			const added = await api.social.addToCollection(res.collection.id, targetOf(work));
			mine = [added.collection, ...mine];
			name = '';
			toasts.success(t('social.collect.created', { name: added.collection.name }));
		} catch (err) {
			toasts.error(socialError(err, t('social.collect.failed')));
		} finally {
			busy = '';
		}
	}
</script>

<Modal bind:open title={t('social.collect.title')} description={t('social.collect.description', { title: work.title })}>
	{#if !ready}
		<p class="text-sm text-muted">{t('social.loading')}</p>
	{:else}
		<div bind:this={list} class="max-h-64 space-y-1 overflow-y-auto">
			{#each mine as c (c.id)}
				<button
					type="button"
					aria-pressed={holds(c)}
					disabled={busy === c.id}
					onclick={() => void toggle(c)}
					class="flex w-full items-center gap-3 rounded-md border px-3 py-2 text-start text-sm transition-colors {holds(c)
						? 'border-accent bg-accent/10 text-foreground'
						: 'border-transparent text-muted hover:bg-surface-hover hover:text-foreground'}"
				>
					<span class="flex-1 truncate">{c.name}</span>
					<span class="font-mono text-[10px] text-muted">{t('social.collect.count', { count: c.items.length })}</span>
					<span class="w-14 text-end font-mono text-[10px] uppercase tracking-[0.18em]">{holds(c) ? t('social.collect.in') : ''}</span>
				</button>
			{:else}
				<p class="text-sm text-muted">{t('social.collect.none')}</p>
			{/each}
		</div>
		<form class="mt-4 flex gap-2" onsubmit={create}>
			<input bind:this={nameInput} bind:value={name} maxlength="80" class={FIELD} placeholder={t('social.collect.newPlaceholder')} aria-label={t('social.collect.newLabel')} />
			<Button type="submit" variant="secondary" loading={busy === 'new'} disabled={!name.trim()}>{t('social.collect.create')} <Kbd>↵</Kbd></Button>
		</form>
	{/if}
	{#snippet footer()}
		<Button variant="ghost" onclick={() => (open = false)}>{t('common.close')} <Kbd>Esc</Kbd></Button>
	{/snippet}
</Modal>
