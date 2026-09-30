<script lang="ts">
	import { DropdownMenu } from 'bits-ui';
	import ChevronsUpDown from '@lucide/svelte/icons/chevrons-up-down';
	import LogOut from '@lucide/svelte/icons/log-out';
	import Settings from '@lucide/svelte/icons/settings';
	import Play from '@lucide/svelte/icons/play';
	import UserRound from '@lucide/svelte/icons/user-round';
	import { goto } from '$app/navigation';
	import { session } from '$lib/auth/session.svelte';
	import Avatar from '$lib/components/profile/Avatar.svelte';
	import { displayName } from '$lib/profile/mascots';

	let {
		side = 'top',
		compact = false
	}: {
		side?: 'top' | 'bottom';
		compact?: boolean;
	} = $props();


	function signOut(): void {
		session.logout();
		void goto('/login');
	}
</script>

<DropdownMenu.Root>
	<DropdownMenu.Trigger
		class={[
			'flex items-center gap-2 rounded-full text-left transition-colors hover:bg-surface-hover focus-visible:outline-2 focus-visible:outline-accent',
			compact ? 'p-1' : 'w-full px-2 py-2'
		].join(' ')}
		aria-label="Account menu"
	>
		<Avatar user={session.user} class="size-8" />
		{#if !compact}
			<span class="min-w-0 flex-1 truncate text-sm text-foreground">{displayName(session.user)}</span>
			<ChevronsUpDown class="size-3.5 shrink-0 text-muted" />
		{/if}
	</DropdownMenu.Trigger>
	<DropdownMenu.Portal>
		<DropdownMenu.Content
			side={side}
			sideOffset={6}
			align="start"
			class="z-50 min-w-60 rounded-md border border-line bg-surface p-1 shadow-xl"
		>
			<div class="flex items-center gap-3 px-2.5 pb-2.5 pt-2">
				<Avatar user={session.user} class="size-10" />
				<div class="min-w-0">
					<p class="truncate text-sm font-semibold text-foreground">{displayName(session.user)}</p>
					<p class="truncate font-mono text-[10px] uppercase tracking-[0.14em] text-muted">@{session.user?.username} · {session.user?.role}</p>
				</div>
			</div>
			<DropdownMenu.Separator class="my-1 h-px bg-hairline" />
			<DropdownMenu.Item
				class="flex cursor-default items-center justify-between gap-2 rounded-sm px-2.5 py-2 text-sm text-foreground outline-none data-[highlighted]:bg-surface-hover"
				onSelect={() => void goto('/settings/profile')}
			>
				<span class="flex items-center gap-2"><UserRound class="size-4 text-muted" /> Profile</span>
				<kbd class="font-mono text-[10px] text-muted">g p</kbd>
			</DropdownMenu.Item>
			<DropdownMenu.Item
				class="flex cursor-default items-center justify-between gap-2 rounded-sm px-2.5 py-2 text-sm text-foreground outline-none data-[highlighted]:bg-surface-hover"
				onSelect={() => void goto('/settings/playback')}
			>
				<span class="flex items-center gap-2"><Play class="size-4 text-muted" /> Playback</span>
				<kbd class="font-mono text-[10px] text-muted">g y</kbd>
			</DropdownMenu.Item>
			<DropdownMenu.Item
				class="flex cursor-default items-center justify-between gap-2 rounded-sm px-2.5 py-2 text-sm text-foreground outline-none data-[highlighted]:bg-surface-hover"
				onSelect={() => void goto('/settings')}
			>
				<span class="flex items-center gap-2"><Settings class="size-4 text-muted" /> All settings</span>
			</DropdownMenu.Item>
			<DropdownMenu.Separator class="my-1 h-px bg-hairline" />
			<DropdownMenu.Item
				class="flex cursor-default items-center gap-2 rounded-sm px-2.5 py-2 text-sm text-danger outline-none data-[highlighted]:bg-danger/10"
				onSelect={signOut}
			>
				<LogOut class="size-4" /> Sign out
			</DropdownMenu.Item>
		</DropdownMenu.Content>
	</DropdownMenu.Portal>
</DropdownMenu.Root>
