<script lang="ts">
	import type { User } from '$lib/api/types';
	import { mediaUrl } from '$lib/api';
	import { session } from '$lib/auth/session.svelte';
	import { displayName } from '$lib/profile/mascots';
	import Mascot from './Mascot.svelte';

	let {
		user,
		class: className = 'size-8'
	}: {
		user: Pick<User, 'id' | 'username' | 'profile'> | null;
		class?: string;
	} = $props();

	const avatar = $derived(user?.profile?.avatar);
	let failed = $state(false);
	const src = $derived(
		user && avatar?.kind === 'upload'
			? mediaUrl(`/api/users/${encodeURIComponent(user.id)}/avatar`, session.token, {
					v: String(avatar.version ?? 0)
				})
			: ''
	);
	$effect(() => {
		void src;
		failed = false;
	});
	const initial = $derived(displayName(user).slice(0, 1).toUpperCase() || '?');
</script>

<span
	class={['relative inline-flex shrink-0 overflow-hidden rounded-full ring-1 ring-white/10', className].join(' ')}
	style="container-type: size;"
>
	{#if src && !failed}
		<img {src} alt="" class="size-full object-cover" decoding="async" onerror={() => (failed = true)} />
	{:else if avatar?.kind === 'mascot' && avatar.mascot}
		<Mascot id={avatar.mascot} class="size-full" />
	{:else}
		<span class="flex size-full items-center justify-center bg-accent/15 font-semibold text-accent" style="font-size: 45cqh;">
			{initial}
		</span>
	{/if}
</span>
