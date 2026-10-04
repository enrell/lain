<script lang="ts">
	/*
	 * One collection (docs/slices/social.md). Owners edit (e), share (s)
	 * and delete (d); people it was shared with read it.
	 */
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import FolderX from '@lucide/svelte/icons/folder-x';
	import { api, ApiError, type Collection, type UserMap, type Visibility, targetOf } from '$lib/api';
	import { session } from '$lib/auth/session.svelte';
	import { t } from '$lib/i18n';
	import Button from '$lib/components/primitives/Button.svelte';
	import EmptyState from '$lib/components/primitives/EmptyState.svelte';
	import ErrorState from '$lib/components/primitives/ErrorState.svelte';
	import Modal from '$lib/components/primitives/Modal.svelte';
	import Skeleton from '$lib/components/primitives/Skeleton.svelte';
	import Kbd from '$lib/components/social/Kbd.svelte';
	import ShareDialog from '$lib/components/social/ShareDialog.svelte';
	import UserChip from '$lib/components/social/UserChip.svelte';
	import WorkLink from '$lib/components/social/WorkLink.svelte';
	import { focusSoon } from '$lib/components/social/focus';
	import { FIELD } from '$lib/components/social/styles';
	import { pageKey, socialError } from '$lib/social/format';
	import { toasts } from '$lib/stores/toasts.svelte';
	import { formatRelative } from '$lib/utilities/format';

	const id = $derived(page.params.id ?? '');

	let col = $state<Collection | null>(null);
	let users = $state<UserMap>({});
	let loadError = $state('');
	let missing = $state(false);
	let editOpen = $state(false);
	let shareOpen = $state(false);
	let deleteOpen = $state(false);
	let saving = $state(false);
	let name = $state('');
	let description = $state('');
	let visibility = $state<Visibility>('private');
	let nameInput = $state<HTMLInputElement>();

	const owner = $derived(!!col && col.owner_id === session.user?.id);

	async function load(collectionId: string): Promise<void> {
		loadError = '';
		missing = false;
		try {
			const res = await api.social.collection(collectionId);
			col = res.collection;
			users = res.users;
		} catch (err) {
			if (err instanceof ApiError && err.kind === 'not-found') missing = true;
			else loadError = socialError(err, t('social.collections.loadFailed'));
		}
	}

	$effect(() => {
		void load(id);
	});

	function openEdit(): void {
		if (!col) return;
		name = col.name;
		description = col.description ?? '';
		visibility = col.visibility;
		editOpen = true;
		focusSoon(() => nameInput);
	}

	async function save(e?: SubmitEvent): Promise<void> {
		e?.preventDefault();
		if (!col || saving) return;
		saving = true;
		try {
			const res = await api.social.updateCollection(col.id, { name, description, visibility });
			col = res.collection;
			users = res.users;
			editOpen = false;
		} catch (err) {
			toasts.error(socialError(err, t('social.collections.saveFailed')));
		} finally {
			saving = false;
		}
	}

	async function remove(): Promise<void> {
		if (!col) return;
		saving = true;
		try {
			await api.social.deleteCollection(col.id);
			await goto('/social?tab=collections');
		} catch (err) {
			toasts.error(socialError(err, t('social.collections.deleteFailed')));
		} finally {
			saving = false;
		}
	}

	async function removeItem(index: number): Promise<void> {
		if (!col) return;
		try {
			const res = await api.social.removeFromCollection(col.id, targetOf(col.items[index].work));
			col = res.collection;
		} catch (err) {
			toasts.error(socialError(err, t('social.collections.saveFailed')));
		}
	}

	async function unshare(userId: string): Promise<void> {
		if (!col) return;
		try {
			const res = await api.social.unshareCollection(col.id, userId);
			col = res.collection;
		} catch (err) {
			toasts.error(socialError(err, t('social.collections.saveFailed')));
		}
	}

	function openDelete(): void {
		deleteOpen = true;
		focusSoon(() => document.getElementById('confirm-delete-collection'));
	}

	function onKey(e: KeyboardEvent): void {
		if (!owner) return;
		const k = pageKey(e);
		if (k === 'e') openEdit();
		else if (k === 's') shareOpen = true;
		else if (k === 'd') openDelete();
		else return;
		e.preventDefault();
	}

	const visLabel = (v: Visibility) => t(`social.visibility.${v}`);
</script>

<svelte:head><title>{t('social.collections.pageTitle', { name: col?.name ?? '' })}</title></svelte:head>
<svelte:window onkeydown={onKey} />

{#if missing}
	<EmptyState title={t('social.collections.missingTitle')} description={t('social.collections.missingBody')}>
		{#snippet icon()}<FolderX class="size-6 text-muted" />{/snippet}
	</EmptyState>
{:else if loadError}
	<ErrorState message={loadError} retry={() => void load(id)} />
{:else if !col}
	<Skeleton class="h-48 w-full" />
{:else}
	<article class="space-y-8">
		<header class="flex flex-wrap items-end justify-between gap-4">
			<div class="min-w-0">
				<p class="font-mono text-[10px] uppercase tracking-[0.26em] text-muted">{t('social.collections.kicker')} · {visLabel(col.visibility)}</p>
				<h1 class="mt-1 text-3xl font-semibold tracking-tight text-foreground">{col.name}</h1>
				{#if col.description}<p class="mt-2 max-w-prose whitespace-pre-line text-sm text-muted">{col.description}</p>{/if}
				<div class="mt-3"><UserChip user={users[col.owner_id]} sub={formatRelative(col.updated_at)} /></div>
			</div>
			{#if owner}
				<div class="flex flex-wrap gap-2">
					<Button size="sm" variant="secondary" onclick={openEdit}>{t('social.collections.edit')} <Kbd>E</Kbd></Button>
					<Button size="sm" variant="secondary" onclick={() => (shareOpen = true)}>{t('social.title.share')} <Kbd>S</Kbd></Button>
					<Button size="sm" variant="ghost" onclick={openDelete}>{t('social.collections.delete')} <Kbd>D</Kbd></Button>
				</div>
			{/if}
		</header>

		<section>
			<h2 class="border-b border-hairline pb-2 font-mono text-[10px] font-semibold uppercase tracking-[0.26em] text-muted">{t('social.collect.count', { count: col.items.length })}</h2>
			<ol>
				{#each col.items as it, i (it.work.kind + it.work.title)}
					<li class="flex flex-wrap items-center gap-x-4 gap-y-1 border-b border-hairline py-3">
						<span class="w-6 font-mono text-[10px] text-muted">{i + 1}</span>
						<span class="min-w-0 flex-1">
							<WorkLink work={it.work} />
							{#if it.note}<span class="block text-sm text-muted">{it.note}</span>{/if}
						</span>
						{#if owner}
							<button type="button" class="font-mono text-[10px] uppercase tracking-[0.16em] text-muted hover:text-danger" onclick={() => void removeItem(i)}>{t('social.collections.removeItem')}</button>
						{/if}
					</li>
				{:else}
					<li class="py-3 text-sm text-muted">{t('social.collections.noItems')}</li>
				{/each}
			</ol>
		</section>

		{#if owner && col.shared_with.length > 0}
			<section>
				<h2 class="border-b border-hairline pb-2 font-mono text-[10px] font-semibold uppercase tracking-[0.26em] text-muted">{t('social.collections.sharedWith')}</h2>
				<ul>
					{#each col.shared_with as uid (uid)}
						<li class="flex items-center justify-between border-b border-hairline py-2.5">
							<UserChip user={users[uid]} />
							<Button size="sm" variant="ghost" onclick={() => void unshare(uid)}>{t('social.collections.unshare')}</Button>
						</li>
					{/each}
				</ul>
			</section>
		{/if}
	</article>

	<Modal bind:open={editOpen} title={t('social.collections.edit')}>
		<form id="edit-collection" class="space-y-3" onsubmit={save}>
			<input bind:this={nameInput} bind:value={name} maxlength="80" class={FIELD} aria-label={t('social.collections.name')} />
			<textarea bind:value={description} maxlength="500" rows="3" class={FIELD} aria-label={t('social.collections.descriptionField')}></textarea>
			<label class="flex items-center justify-between gap-3 text-sm text-muted">
				{t('social.collections.visibility')}
				<select bind:value={visibility} class="rounded-md bg-field px-2 py-1.5 text-sm text-foreground">
					<option value="private">{visLabel('private')}</option>
					<option value="friends">{visLabel('friends')}</option>
					<option value="public">{visLabel('public')}</option>
				</select>
			</label>
		</form>
		{#snippet footer()}
			<Button variant="ghost" onclick={() => (editOpen = false)}>{t('common.cancel')} <Kbd>Esc</Kbd></Button>
			<Button type="submit" form="edit-collection" loading={saving} disabled={!name.trim()}>{t('social.collections.save')} <Kbd>↵</Kbd></Button>
		{/snippet}
	</Modal>

	<Modal bind:open={deleteOpen} title={t('social.collections.deleteTitle', { name: col.name })} description={t('social.collections.deleteBody')}>
		{#snippet footer()}
			<Button variant="ghost" onclick={() => (deleteOpen = false)}>{t('common.cancel')} <Kbd>Esc</Kbd></Button>
			<Button id="confirm-delete-collection" variant="danger" loading={saving} onclick={() => void remove()}>{t('social.collections.delete')} <Kbd>↵</Kbd></Button>
		{/snippet}
	</Modal>

	<ShareDialog
		bind:open={shareOpen}
		heading={t('social.share.collectionTitle', { name: col.name })}
		onSend={async (to, message) => {
			if (!col) return;
			await api.social.shareCollection(col.id, to, message);
			await load(col.id);
		}}
	/>
{/if}
