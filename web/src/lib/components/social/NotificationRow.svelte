<script lang="ts">
	import { api, type Notification as SocialNotification, type UserMap } from '$lib/api';
	import { t } from '$lib/i18n';
	import Button from '$lib/components/primitives/Button.svelte';
	import { socialError } from '$lib/social/format';
	import { toasts } from '$lib/stores/toasts.svelte';
	import { formatRelative } from '$lib/utilities/format';
	import UserChip from './UserChip.svelte';
	import WorkLink from './WorkLink.svelte';

	let {
		n,
		users,
		onChange
	}: { n: SocialNotification; users: UserMap; onChange: () => void } = $props();

	let busy = $state(false);

	const label = $derived.by(() => {
		switch (n.type) {
			case 'friend_request':
				return t('social.notify.friendRequest');
			case 'friend_accepted':
				return t('social.notify.friendAccepted');
			case 'share':
				return t('social.notify.share');
			case 'collection_shared':
				return t('social.notify.collectionShared');
			default:
				return t('social.notify.reply');
		}
	});

	async function act(fn: () => Promise<unknown>): Promise<void> {
		busy = true;
		try {
			await fn();
			if (!n.read) await api.social.markRead({ ids: [n.id] });
			onChange();
		} catch (err) {
			toasts.error(socialError(err, t('social.notify.failed')));
		} finally {
			busy = false;
		}
	}
</script>

<li class="flex flex-wrap items-center gap-x-4 gap-y-2 border-b border-hairline py-3 {n.read ? 'opacity-70' : ''}">
	<span class="size-1.5 shrink-0 rounded-full {n.read ? 'bg-transparent' : 'bg-accent'}" aria-hidden="true"></span>
	<span class="w-44 min-w-0 shrink-0"><UserChip user={users[n.from_user_id]} /></span>
	<span class="font-mono text-[10px] uppercase tracking-[0.2em] text-accent">{label}</span>
	<span class="min-w-0 flex-1">
		{#if n.work}
			<WorkLink work={n.work} />
		{:else if n.collection_id}
			<a class="font-medium text-foreground hover:text-accent" href="/social/collections/{encodeURIComponent(n.collection_id)}">{t('social.notify.openCollection')}</a>
		{/if}
		{#if n.message}<span class="block truncate text-sm text-muted">“{n.message}”</span>{/if}
	</span>
	{#if n.type === 'friend_request' && !n.read && users[n.from_user_id]}
		<span class="flex gap-2">
			<Button size="sm" disabled={busy} onclick={() => void act(() => api.social.befriend(n.from_user_id))}>{t('social.friends.accept')}</Button>
			<Button size="sm" variant="ghost" disabled={busy} onclick={() => void act(() => api.social.decline(n.from_user_id))}>{t('social.friends.decline')}</Button>
		</span>
	{/if}
	<time class="ms-auto shrink-0 font-mono text-[10px] text-muted">{formatRelative(n.at)}</time>
</li>
