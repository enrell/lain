<script lang="ts">
	/*
	 * A member's profile as the viewer may see it (docs/slices/social.md).
	 * Keys: a add/accept friend, x remove/cancel, b block/unblock.
	 */
	import { page } from '$app/state';
	import UserX from '@lucide/svelte/icons/user-x';
	import { api, ApiError, type Activity, type Collection, type ProfileView, type Rating, type Relation } from '$lib/api';
	import { t } from '$lib/i18n';
	import Avatar from '$lib/components/profile/Avatar.svelte';
	import Button from '$lib/components/primitives/Button.svelte';
	import EmptyState from '$lib/components/primitives/EmptyState.svelte';
	import ErrorState from '$lib/components/primitives/ErrorState.svelte';
	import Modal from '$lib/components/primitives/Modal.svelte';
	import Skeleton from '$lib/components/primitives/Skeleton.svelte';
	import ActivityRow from '$lib/components/social/ActivityRow.svelte';
	import Kbd from '$lib/components/social/Kbd.svelte';
	import WorkLink from '$lib/components/social/WorkLink.svelte';
	import { focusSoon } from '$lib/components/social/focus';
	import { asAvatarUser, pageKey, socialError, userName } from '$lib/social/format';
	import { toasts } from '$lib/stores/toasts.svelte';
	import { formatRelative } from '$lib/utilities/format';

	const id = $derived(page.params.id ?? '');

	let profile = $state<ProfileView | null>(null);
	let activity = $state<Activity[]>([]);
	let ratings = $state<Rating[]>([]);
	let collections = $state<Collection[]>([]);
	let loadError = $state('');
	let missing = $state(false);
	let busy = $state(false);
	let confirmBlock = $state(false);

	async function load(userId: string): Promise<void> {
		loadError = '';
		missing = false;
		profile = null;
		try {
			const p = await api.social.profile(userId);
			const [a, r, c] = await Promise.all([
				p.visible.activity ? api.social.userActivity(userId) : Promise.resolve({ items: [] as Activity[] }),
				p.visible.ratings ? api.social.userRatings(userId) : Promise.resolve({ ratings: [] as Rating[] }),
				p.visible.profile ? api.social.userCollections(userId) : Promise.resolve({ collections: [] as Collection[] })
			]);
			activity = a.items;
			ratings = r.ratings;
			collections = c.collections;
			profile = p;
		} catch (err) {
			if (err instanceof ApiError && err.kind === 'not-found') missing = true;
			else loadError = socialError(err, t('social.profile.loadFailed'));
		}
	}

	$effect(() => {
		void load(id);
	});

	async function run(fn: (id: string) => Promise<{ relation: Relation }>): Promise<void> {
		if (!profile || busy) return;
		busy = true;
		try {
			const res = await fn(profile.user.id);
			// A relation change can change what is visible; reload.
			if (res.relation !== profile.relation) await load(profile.user.id);
		} catch (err) {
			toasts.error(socialError(err, t('social.friends.failed')));
		} finally {
			busy = false;
		}
	}

	const primary = $derived.by(() => {
		switch (profile?.relation) {
			case 'none':
				return { label: t('social.friends.add'), fn: api.social.befriend };
			case 'incoming':
				return { label: t('social.friends.accept'), fn: api.social.befriend };
			default:
				return null;
		}
	});
	const secondary = $derived.by(() => {
		switch (profile?.relation) {
			case 'friend':
				return { label: t('social.friends.remove'), fn: api.social.unfriend };
			case 'outgoing':
				return { label: t('social.friends.cancel'), fn: api.social.unfriend };
			case 'incoming':
				return { label: t('social.friends.decline'), fn: api.social.decline };
			default:
				return null;
		}
	});

	function askBlock(): void {
		if (profile?.relation === 'blocking') {
			void run(api.social.unblock);
			return;
		}
		confirmBlock = true;
		focusSoon(() => document.getElementById('confirm-block'));
	}

	function onKey(e: KeyboardEvent): void {
		if (!profile || profile.self) return;
		const k = pageKey(e);
		if (k === 'a' && primary) void run(primary.fn);
		else if (k === 'x' && secondary) void run(secondary.fn);
		else if (k === 'b') askBlock();
		else return;
		e.preventDefault();
	}

	const relationLabel = (r: Relation) => t(`social.relation.${r}`);
</script>

<svelte:head><title>{t('social.profile.pageTitle', { name: profile ? userName(profile.user) : '' })}</title></svelte:head>
<svelte:window onkeydown={onKey} />

{#if missing}
	<EmptyState title={t('social.profile.missingTitle')} description={t('social.profile.missingBody')}>
		{#snippet icon()}<UserX class="size-6 text-muted" />{/snippet}
	</EmptyState>
{:else if loadError}
	<ErrorState message={loadError} retry={() => void load(id)} />
{:else if !profile}
	<div class="space-y-4">
		<Skeleton class="h-24 w-full" />
		<Skeleton class="h-48 w-full" />
	</div>
{:else}
	<article class="space-y-10">
		<header class="flex flex-wrap items-center gap-6">
			<Avatar user={asAvatarUser(profile.user)} class="size-20" />
			<div class="min-w-0 flex-1">
				<h1 class="truncate text-3xl font-semibold tracking-tight text-foreground">{userName(profile.user)}</h1>
				<p class="mt-1 flex flex-wrap items-center gap-3 font-mono text-[11px] text-muted">
					<span>@{profile.user.username}</span>
					{#if !profile.self && profile.relation !== 'none'}
						<span class="uppercase tracking-[0.2em] text-accent">{relationLabel(profile.relation)}</span>
					{/if}
				</p>
				{#if profile.bio}<p class="mt-3 max-w-prose whitespace-pre-line text-sm leading-6 text-foreground/90">{profile.bio}</p>{/if}
			</div>
			{#if profile.self}
				<a href="/settings/profile" class="font-mono text-[10px] uppercase tracking-[0.2em] text-muted hover:text-foreground">{t('social.profile.edit')}</a>
			{:else}
				<div class="flex flex-wrap gap-2">
					{#if primary}
						<Button size="sm" disabled={busy} onclick={() => void run(primary.fn)}>{primary.label} <Kbd>A</Kbd></Button>
					{/if}
					{#if secondary}
						<Button size="sm" variant="secondary" disabled={busy} onclick={() => void run(secondary.fn)}>{secondary.label} <Kbd>X</Kbd></Button>
					{/if}
					<Button size="sm" variant="ghost" disabled={busy} onclick={askBlock}>
						{profile.relation === 'blocking' ? t('social.friends.unblock') : t('social.friends.block')} <Kbd>B</Kbd>
					</Button>
				</div>
			{/if}
		</header>

		<section>
			<h2 class="border-b border-hairline pb-2 font-mono text-[10px] font-semibold uppercase tracking-[0.26em] text-muted">{t('social.profile.favorites')}</h2>
			{#if !profile.visible.profile}
				<p class="py-3 text-sm text-muted">{t('social.profile.private')}</p>
			{:else if profile.favorites.length === 0}
				<p class="py-3 text-sm text-muted">{t('social.profile.noFavorites')}</p>
			{:else}
				<ul class="flex flex-wrap gap-x-6 gap-y-2 py-3">
					{#each profile.favorites as w (w.kind + w.title)}
						<li><WorkLink work={w} /></li>
					{/each}
				</ul>
			{/if}
		</section>

		<section>
			<h2 class="border-b border-hairline pb-2 font-mono text-[10px] font-semibold uppercase tracking-[0.26em] text-muted">{t('social.profile.activity')}</h2>
			{#if !profile.visible.activity}
				<p class="py-3 text-sm text-muted">{t('social.profile.private')}</p>
			{:else}
				<ul>
					{#each activity as a (a.id)}
						<ActivityRow activity={a} showUser={false} />
					{:else}
						<li class="py-3 text-sm text-muted">{t('social.profile.noActivity')}</li>
					{/each}
				</ul>
			{/if}
		</section>

		<section>
			<h2 class="border-b border-hairline pb-2 font-mono text-[10px] font-semibold uppercase tracking-[0.26em] text-muted">{t('social.profile.ratings')}</h2>
			{#if !profile.visible.ratings}
				<p class="py-3 text-sm text-muted">{t('social.profile.private')}</p>
			{:else}
				<ul>
					{#each ratings as r (r.work.kind + r.work.title)}
						<li class="border-b border-hairline py-3">
							<div class="flex items-center justify-between gap-3">
								<WorkLink work={r.work} />
								<span class="flex items-center gap-3">
									{#if r.score}<span class="font-mono text-sm text-foreground">{t('social.score', { score: r.score })}</span>{/if}
									<time class="font-mono text-[10px] text-muted">{formatRelative(r.updated_at)}</time>
								</span>
							</div>
							{#if r.review && !r.spoiler}<p class="mt-2 whitespace-pre-line text-sm leading-6 text-foreground/90">{r.review}</p>{/if}
							{#if r.review && r.spoiler}<p class="mt-2 font-mono text-[11px] text-muted">{t('social.profile.spoilerHidden')}</p>{/if}
						</li>
					{:else}
						<li class="py-3 text-sm text-muted">{t('social.profile.noRatings')}</li>
					{/each}
				</ul>
			{/if}
		</section>

		<section>
			<h2 class="border-b border-hairline pb-2 font-mono text-[10px] font-semibold uppercase tracking-[0.26em] text-muted">{t('social.profile.collections')}</h2>
			{#if !profile.visible.profile}
				<p class="py-3 text-sm text-muted">{t('social.profile.private')}</p>
			{:else}
				<ul>
					{#each collections as c (c.id)}
						<li class="border-b border-hairline py-3">
							<a href="/social/collections/{encodeURIComponent(c.id)}" class="flex items-baseline gap-4 hover:text-accent">
								<span class="font-medium text-foreground">{c.name}</span>
								<span class="font-mono text-[10px] text-muted">{t('social.collect.count', { count: c.items.length })}</span>
							</a>
						</li>
					{:else}
						<li class="py-3 text-sm text-muted">{t('social.profile.noCollections')}</li>
					{/each}
				</ul>
			{/if}
		</section>
	</article>

	<Modal bind:open={confirmBlock} title={t('social.block.title', { name: userName(profile.user) })} description={t('social.block.description')}>
		{#snippet footer()}
			<Button variant="ghost" onclick={() => (confirmBlock = false)}>{t('common.cancel')} <Kbd>Esc</Kbd></Button>
			<Button
				id="confirm-block"
				variant="danger"
				disabled={busy}
				onclick={async () => {
					confirmBlock = false;
					await run(api.social.block);
				}}
			>
				{t('social.friends.block')} <Kbd>↵</Kbd>
			</Button>
		{/snippet}
	</Modal>
{/if}
