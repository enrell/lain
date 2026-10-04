<script lang="ts">
	/*
	 * Social hub (docs/slices/social.md): feed, friends, notifications and
	 * collections. Keys: 1–4 switch tabs, / searches people, r marks
	 * notifications read, n starts a collection.
	 */
	import { tick } from 'svelte';
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import Users from '@lucide/svelte/icons/users';
	import {
		api,
		type Activity,
		type Collection,
		type FriendsView,
		type Notification as SocialNotification,
		type Relation,
		type UserMap,
		type UserSummary,
		type Visibility
	} from '$lib/api';
	import { session } from '$lib/auth/session.svelte';
	import { t } from '$lib/i18n';
	import Button from '$lib/components/primitives/Button.svelte';
	import EmptyState from '$lib/components/primitives/EmptyState.svelte';
	import ErrorState from '$lib/components/primitives/ErrorState.svelte';
	import Modal from '$lib/components/primitives/Modal.svelte';
	import Skeleton from '$lib/components/primitives/Skeleton.svelte';
	import ActivityRow from '$lib/components/social/ActivityRow.svelte';
	import Kbd from '$lib/components/social/Kbd.svelte';
	import NotificationRow from '$lib/components/social/NotificationRow.svelte';
	import UserChip from '$lib/components/social/UserChip.svelte';
	import { focusSoon } from '$lib/components/social/focus';
	import { FIELD } from '$lib/components/social/styles';
	import { pageKey, socialError } from '$lib/social/format';
	import { socialBadge } from '$lib/stores/social.svelte';
	import { toasts } from '$lib/stores/toasts.svelte';
	import { debounce } from '$lib/utilities/debounce';
	import { formatRelative } from '$lib/utilities/format';

	type Tab = 'feed' | 'friends' | 'notifications' | 'collections';
	const TABS: { id: Tab; key: string; label: () => string }[] = [
		{ id: 'feed', key: '1', label: () => t('social.tab.feed') },
		{ id: 'friends', key: '2', label: () => t('social.tab.friends') },
		{ id: 'notifications', key: '3', label: () => t('social.tab.notifications') },
		{ id: 'collections', key: '4', label: () => t('social.tab.collections') }
	];

	const tab = $derived<Tab>((TABS.find((x) => x.id === page.url.searchParams.get('tab'))?.id ?? 'feed') as Tab);

	let loadError = $state('');
	let ready = $state(false);

	// Feed
	let feed = $state<Activity[]>([]);
	let feedUsers = $state<UserMap>({});
	let feedMore = $state(true);
	let feedBusy = $state(false);

	// Friends + search
	let friends = $state<FriendsView | null>(null);
	let query = $state('');
	let results = $state<{ user: UserSummary; relation: Relation }[]>([]);
	let searchInput = $state<HTMLInputElement>();
	let busyUser = $state('');

	// Notifications
	let notes = $state<SocialNotification[]>([]);
	let noteUsers = $state<UserMap>({});
	let unread = $state(0);

	// Collections
	let collections = $state<Collection[]>([]);
	let collectionUsers = $state<UserMap>({});
	let newOpen = $state(false);
	let newName = $state('');
	let newDesc = $state('');
	let newVis = $state<Visibility>('private');
	let creating = $state(false);
	let newInput = $state<HTMLInputElement>();

	async function load(which: Tab): Promise<void> {
		loadError = '';
		ready = false;
		try {
			if (which === 'feed') {
				const res = await api.social.feed();
				feed = res.items;
				feedUsers = res.users;
				feedMore = res.items.length > 0;
			} else if (which === 'friends') {
				friends = await api.social.friends();
				await search();
			} else if (which === 'notifications') {
				await loadNotes();
			} else {
				const res = await api.social.collections();
				collections = res.collections;
				collectionUsers = res.users;
			}
		} catch (err) {
			loadError = socialError(err, t('social.loadFailed'));
		} finally {
			ready = true;
		}
	}

	$effect(() => {
		void load(tab);
	});

	async function loadNotes(): Promise<void> {
		const res = await api.social.notifications();
		notes = res.items;
		noteUsers = res.users;
		unread = res.unread;
		socialBadge.unread = res.unread;
	}

	async function moreFeed(): Promise<void> {
		const last = feed.at(-1);
		if (!last || feedBusy) return;
		feedBusy = true;
		try {
			const res = await api.social.feed(last.at);
			feed = [...feed, ...res.items];
			feedUsers = { ...feedUsers, ...res.users };
			feedMore = res.items.length > 0;
		} catch (err) {
			toasts.error(socialError(err, t('social.loadFailed')));
		} finally {
			feedBusy = false;
		}
	}

	async function search(): Promise<void> {
		try {
			results = (await api.social.searchUsers(query.trim())).users;
		} catch (err) {
			toasts.error(socialError(err, t('social.friends.searchFailed')));
		}
	}
	const searchSoon = debounce(() => void search(), 200);

	async function relate(id: string, fn: (id: string) => Promise<unknown>): Promise<void> {
		busyUser = id;
		try {
			await fn(id);
			friends = await api.social.friends();
			await search();
			void socialBadge.refresh();
		} catch (err) {
			toasts.error(socialError(err, t('social.friends.failed')));
		} finally {
			busyUser = '';
		}
	}

	async function markAllRead(): Promise<void> {
		try {
			await api.social.markRead({ all: true });
			await loadNotes();
		} catch (err) {
			toasts.error(socialError(err, t('social.notify.failed')));
		}
	}

	function openNew(): void {
		newName = '';
		newDesc = '';
		newVis = 'private';
		newOpen = true;
		focusSoon(() => newInput);
	}

	async function createCollection(e?: SubmitEvent): Promise<void> {
		e?.preventDefault();
		if (!newName.trim() || creating) return;
		creating = true;
		try {
			const res = await api.social.createCollection({ name: newName, description: newDesc, visibility: newVis });
			newOpen = false;
			await goto(`/social/collections/${encodeURIComponent(res.collection.id)}`);
		} catch (err) {
			toasts.error(socialError(err, t('social.collections.createFailed')));
		} finally {
			creating = false;
		}
	}

	function setTab(id: Tab): void {
		const url = new URL(page.url);
		url.searchParams.set('tab', id);
		void goto(url, { replaceState: true, keepFocus: true, noScroll: true });
	}

	async function onKey(e: KeyboardEvent): Promise<void> {
		const k = pageKey(e);
		if (!k) return;
		const target = TABS.find((x) => x.key === k);
		if (target) {
			e.preventDefault();
			setTab(target.id);
		} else if (k === '/') {
			e.preventDefault();
			setTab('friends');
			await tick();
			searchInput?.focus();
		} else if (k === 'r' && tab === 'notifications') {
			e.preventDefault();
			void markAllRead();
		} else if (k === 'n' && tab === 'collections') {
			e.preventDefault();
			openNew();
		}
	}

	const relationAction = (rel: Relation) => {
		switch (rel) {
			case 'none':
				return { label: t('social.friends.add'), fn: api.social.befriend };
			case 'incoming':
				return { label: t('social.friends.accept'), fn: api.social.befriend };
			case 'outgoing':
				return { label: t('social.friends.cancel'), fn: api.social.unfriend };
			default:
				return null;
		}
	};
	const visLabel = (v: Visibility) => t(`social.visibility.${v}`);
</script>

<svelte:head><title>{t('social.pageTitle')}</title></svelte:head>
<svelte:window onkeydown={(e) => void onKey(e)} />

<div class="space-y-6">
	<header class="flex flex-wrap items-end justify-between gap-4">
		<div>
			<h1 class="text-xl font-semibold tracking-tight text-foreground">{t('social.heading')}</h1>
			<p class="mt-0.5 text-sm text-muted">{t('social.subtitle')}</p>
		</div>
		<a href="/settings/privacy" class="font-mono text-[10px] uppercase tracking-[0.2em] text-muted hover:text-foreground">{t('social.privacyLink')}</a>
	</header>

	<div class="flex flex-wrap gap-6 border-b border-hairline" role="tablist" aria-label={t('social.heading')}>
		{#each TABS as x (x.id)}
			<button
				type="button"
				role="tab"
				aria-selected={tab === x.id}
				onclick={() => setTab(x.id)}
				class="-mb-px flex items-center gap-2 border-b-2 pb-2 text-sm transition-colors {tab === x.id
					? 'border-accent text-foreground'
					: 'border-transparent text-muted hover:text-foreground'}"
			>
				{x.label()}
				{#if x.id === 'notifications' && socialBadge.unread > 0}
					<span class="rounded-full bg-accent px-1.5 font-mono text-[10px] text-background">{socialBadge.unread}</span>
				{/if}
				<Kbd>{x.key}</Kbd>
			</button>
		{/each}
	</div>

	{#if loadError}
		<ErrorState message={loadError} retry={() => void load(tab)} />
	{:else if !ready}
		<div class="space-y-3">
			{#each Array(6) as _}<Skeleton class="h-10 w-full" />{/each}
		</div>
	{:else if tab === 'feed'}
		{#if feed.length === 0}
			<EmptyState title={t('social.feed.emptyTitle')} description={t('social.feed.emptyBody')}>
				{#snippet icon()}<Users class="size-6 text-muted" />{/snippet}
				<Button variant="secondary" onclick={() => setTab('friends')}>{t('social.feed.findFriends')} <Kbd>/</Kbd></Button>
			</EmptyState>
		{:else}
			<ul>
				{#each feed as a (a.id)}
					<ActivityRow activity={a} user={feedUsers[a.user_id]} />
				{/each}
			</ul>
			{#if feedMore}
				<Button variant="ghost" loading={feedBusy} onclick={() => void moreFeed()}>{t('social.feed.more')}</Button>
			{/if}
		{/if}
	{:else if tab === 'friends' && friends}
		<section class="space-y-3">
			<label class="block">
				<span class="sr-only">{t('social.friends.searchLabel')}</span>
				<input
					bind:this={searchInput}
					bind:value={query}
					oninput={searchSoon}
					onkeydown={(e) => e.key === 'Escape' && searchInput?.blur()}
					class={FIELD}
					placeholder={t('social.friends.searchPlaceholder')}
				/>
			</label>
			<ul>
				{#each results as r (r.user.id)}
					{@const action = relationAction(r.relation)}
					<li class="flex items-center justify-between gap-3 border-b border-hairline py-2.5">
						<UserChip user={r.user} sub={'@' + r.user.username} />
						{#if action}
							<Button size="sm" variant={r.relation === 'outgoing' ? 'ghost' : 'secondary'} disabled={busyUser === r.user.id} onclick={() => void relate(r.user.id, action.fn)}>{action.label}</Button>
						{:else}
							<span class="font-mono text-[10px] uppercase tracking-[0.18em] text-muted">{t('social.relation.friend')}</span>
						{/if}
					</li>
				{:else}
					<li class="py-3 text-sm text-muted">{t('social.friends.noResults')}</li>
				{/each}
			</ul>
		</section>

		{#if friends.incoming.length > 0}
			<section>
				<h2 class="border-b border-hairline pb-2 font-mono text-[10px] font-semibold uppercase tracking-[0.26em] text-muted">{t('social.friends.incoming')}</h2>
				<ul>
					{#each friends.incoming as r (r.user.id)}
						<li class="flex items-center justify-between gap-3 border-b border-hairline py-2.5">
							<UserChip user={r.user} sub={formatRelative(r.since)} />
							<span class="flex gap-2">
								<Button size="sm" disabled={busyUser === r.user.id} onclick={() => void relate(r.user.id, api.social.befriend)}>{t('social.friends.accept')}</Button>
								<Button size="sm" variant="ghost" disabled={busyUser === r.user.id} onclick={() => void relate(r.user.id, api.social.decline)}>{t('social.friends.decline')}</Button>
							</span>
						</li>
					{/each}
				</ul>
			</section>
		{/if}

		<section>
			<h2 class="border-b border-hairline pb-2 font-mono text-[10px] font-semibold uppercase tracking-[0.26em] text-muted">{t('social.friends.list', { count: friends.friends.length })}</h2>
			<ul>
				{#each friends.friends as r (r.user.id)}
					<li class="flex items-center justify-between gap-3 border-b border-hairline py-2.5">
						<UserChip user={r.user} sub={t('social.friends.since', { when: formatRelative(r.since) })} />
						<Button size="sm" variant="ghost" disabled={busyUser === r.user.id} onclick={() => void relate(r.user.id, api.social.unfriend)}>{t('social.friends.remove')}</Button>
					</li>
				{:else}
					<li class="py-3 text-sm text-muted">{t('social.friends.none')}</li>
				{/each}
			</ul>
		</section>

		{#if friends.outgoing.length > 0}
			<section>
				<h2 class="border-b border-hairline pb-2 font-mono text-[10px] font-semibold uppercase tracking-[0.26em] text-muted">{t('social.friends.outgoing')}</h2>
				<ul>
					{#each friends.outgoing as r (r.user.id)}
						<li class="flex items-center justify-between gap-3 border-b border-hairline py-2.5">
							<UserChip user={r.user} sub={formatRelative(r.since)} />
							<Button size="sm" variant="ghost" disabled={busyUser === r.user.id} onclick={() => void relate(r.user.id, api.social.unfriend)}>{t('social.friends.cancel')}</Button>
						</li>
					{/each}
				</ul>
			</section>
		{/if}

		{#if friends.blocked.length > 0}
			<section>
				<h2 class="border-b border-hairline pb-2 font-mono text-[10px] font-semibold uppercase tracking-[0.26em] text-muted">{t('social.friends.blocked')}</h2>
				<ul>
					{#each friends.blocked as r (r.user.id)}
						<li class="flex items-center justify-between gap-3 border-b border-hairline py-2.5">
							<UserChip user={r.user} />
							<Button size="sm" variant="ghost" disabled={busyUser === r.user.id} onclick={() => void relate(r.user.id, api.social.unblock)}>{t('social.friends.unblock')}</Button>
						</li>
					{/each}
				</ul>
			</section>
		{/if}
	{:else if tab === 'notifications'}
		<div class="flex items-center justify-between">
			<p class="text-sm text-muted">{t('social.notify.unread', { count: unread })}</p>
			<Button size="sm" variant="ghost" disabled={unread === 0} onclick={() => void markAllRead()}>{t('social.notify.markAll')} <Kbd>R</Kbd></Button>
		</div>
		<ul>
			{#each notes as n (n.id)}
				<NotificationRow {n} users={noteUsers} onChange={() => void loadNotes()} />
			{:else}
				<li class="py-3 text-sm text-muted">{t('social.notify.empty')}</li>
			{/each}
		</ul>
	{:else if tab === 'collections'}
		<div class="flex justify-end">
			<Button size="sm" onclick={openNew}>{t('social.collections.new')} <Kbd>N</Kbd></Button>
		</div>
		<ul>
			{#each collections as c (c.id)}
				<li class="border-b border-hairline py-3">
					<a href="/social/collections/{encodeURIComponent(c.id)}" class="flex flex-wrap items-baseline gap-x-4 gap-y-1 hover:text-accent">
						<span class="font-medium text-foreground">{c.name}</span>
						<span class="font-mono text-[10px] text-muted">{t('social.collect.count', { count: c.items.length })}</span>
						<span class="font-mono text-[9px] uppercase tracking-[0.18em] text-muted">{visLabel(c.visibility)}</span>
						{#if c.owner_id !== session.user?.id}
							<span class="ms-auto text-xs text-muted">{t('social.collections.sharedBy', { name: collectionUsers[c.owner_id]?.display_name || collectionUsers[c.owner_id]?.username || '' })}</span>
						{/if}
					</a>
				</li>
			{:else}
				<li class="py-3 text-sm text-muted">{t('social.collections.empty')}</li>
			{/each}
		</ul>
	{/if}
</div>

<Modal bind:open={newOpen} title={t('social.collections.new')} description={t('social.collections.newDescription')}>
	<form id="new-collection" class="space-y-3" onsubmit={createCollection}>
		<input bind:this={newInput} bind:value={newName} maxlength="80" class={FIELD} placeholder={t('social.collections.name')} aria-label={t('social.collections.name')} />
		<textarea bind:value={newDesc} maxlength="500" rows="3" class={FIELD} placeholder={t('social.collections.descriptionField')} aria-label={t('social.collections.descriptionField')}></textarea>
		<label class="flex items-center justify-between gap-3 text-sm text-muted">
			{t('social.collections.visibility')}
			<select bind:value={newVis} class="rounded-md bg-field px-2 py-1.5 text-sm text-foreground">
				<option value="private">{visLabel('private')}</option>
				<option value="friends">{visLabel('friends')}</option>
				<option value="public">{visLabel('public')}</option>
			</select>
		</label>
	</form>
	{#snippet footer()}
		<Button variant="ghost" onclick={() => (newOpen = false)}>{t('common.cancel')} <Kbd>Esc</Kbd></Button>
		<Button type="submit" form="new-collection" loading={creating} disabled={!newName.trim()}>{t('social.collections.create')} <Kbd>↵</Kbd></Button>
	{/snippet}
</Modal>
