<script lang="ts">
	import { DropdownMenu } from 'bits-ui';
	import ChevronsUpDown from '@lucide/svelte/icons/chevrons-up-down';
	import LogOut from '@lucide/svelte/icons/log-out';
	import Settings from '@lucide/svelte/icons/settings';
	import UserRound from '@lucide/svelte/icons/user-round';
	import { goto } from '$app/navigation';
	import { session } from '$lib/auth/session.svelte';

	let {
		side = 'top',
		compact = false
	}: {
		side?: 'top' | 'bottom';
		compact?: boolean;
	} = $props();

	const initial = $derived((session.user?.username ?? '?').slice(0, 1).toUpperCase());

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
		<span
			class="flex size-7 shrink-0 items-center justify-center rounded-full bg-accent/15 text-xs font-semibold text-accent"
		>
			{initial}
		</span>
		{#if !compact}
			<span class="min-w-0 flex-1 truncate text-sm text-foreground">{session.user?.username}</span>
			<ChevronsUpDown class="size-3.5 shrink-0 text-muted" />
		{/if}
	</DropdownMenu.Trigger>
	<DropdownMenu.Portal>
		<DropdownMenu.Content
			side={side}
			sideOffset={6}
			align="start"
			class="z-50 min-w-52 rounded-md border border-line bg-surface p-1 shadow-xl"
		>
			<DropdownMenu.Item
				class="flex cursor-default items-center gap-2 rounded-sm px-2.5 py-2 text-sm text-foreground outline-none data-[highlighted]:bg-surface-hover"
				onSelect={() => void goto('/settings')}
			>
				<UserRound class="size-4 text-muted" /> Account
			</DropdownMenu.Item>
			<DropdownMenu.Item
				class="flex cursor-default items-center gap-2 rounded-sm px-2.5 py-2 text-sm text-foreground outline-none data-[highlighted]:bg-surface-hover"
				onSelect={() => void goto('/settings/libraries')}
			>
				<Settings class="size-4 text-muted" /> Settings
			</DropdownMenu.Item>
			<DropdownMenu.Separator class="my-1 h-px bg-line" />
			<DropdownMenu.Item
				class="flex cursor-default items-center gap-2 rounded-sm px-2.5 py-2 text-sm text-danger outline-none data-[highlighted]:bg-danger/10"
				onSelect={signOut}
			>
				<LogOut class="size-4" /> Sign out
			</DropdownMenu.Item>
		</DropdownMenu.Content>
	</DropdownMenu.Portal>
</DropdownMenu.Root>
