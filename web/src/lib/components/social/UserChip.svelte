<script lang="ts">
	import type { UserSummary } from '$lib/api';
	import Avatar from '$lib/components/profile/Avatar.svelte';
	import { asAvatarUser, userName } from '$lib/social/format';

	let {
		user,
		size = 'size-6',
		sub
	}: {
		user: UserSummary | undefined;
		size?: string;
		/** Secondary line (username, time) under the name. */
		sub?: string;
	} = $props();
</script>

{#if user}
	<a href="/u/{encodeURIComponent(user.id)}" class="group inline-flex min-w-0 items-center gap-2.5">
		<Avatar user={asAvatarUser(user)} class={size} />
		<span class="min-w-0">
			<span class="block truncate text-sm font-medium text-foreground group-hover:text-accent">{userName(user)}</span>
			{#if sub}<span class="block truncate font-mono text-[10px] text-muted">{sub}</span>{/if}
		</span>
	</a>
{:else}
	<span class="inline-flex items-center gap-2.5 text-sm text-muted">
		<span class="{size} rounded-full bg-surface-active"></span>{userName(undefined)}
	</span>
{/if}
