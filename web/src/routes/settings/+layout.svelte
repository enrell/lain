<script lang="ts">
	import { page } from '$app/state';
	import Blocks from '@lucide/svelte/icons/blocks';
	import DatabaseBackup from '@lucide/svelte/icons/database-backup';
	import Library from '@lucide/svelte/icons/library';
	import UserRound from '@lucide/svelte/icons/user-round';
	import Users from '@lucide/svelte/icons/users';
	import type { Snippet } from 'svelte';
	import { session } from '$lib/auth/session.svelte';

	let { children }: { children: Snippet } = $props();

	const tabs = $derived(
		[
			{ href: '/settings', label: 'Account', icon: UserRound, admin: false },
			{ href: '/settings/libraries', label: 'Libraries', icon: Library, admin: true },
			{ href: '/settings/users', label: 'Users', icon: Users, admin: true },
			{ href: '/settings/plugins', label: 'Plugins', icon: Blocks, admin: true },
			{ href: '/settings/backup', label: 'Backup', icon: DatabaseBackup, admin: true }
		].filter((tab) => !tab.admin || session.isAdmin)
	);

	function active(href: string): boolean {
		const path = page.url.pathname;
		if (href === '/settings') return path === '/settings';
		return path === href || path.startsWith(href + '/');
	}
</script>

<div class="space-y-6">
	<header>
		<h1 class="text-xl font-semibold tracking-tight text-foreground">Settings</h1>
		<p class="mt-0.5 text-sm text-muted">
			{session.isAdmin ? 'Account, server and composition controls.' : 'Your account.'}
		</p>
	</header>

	<nav class="flex flex-wrap gap-1 border-b border-line" aria-label="Settings sections">
		{#each tabs as tab (tab.href)}
			{@const isActive = active(tab.href)}
			{@const Icon = tab.icon}
			<a
				href={tab.href}
				aria-current={isActive ? 'page' : undefined}
				class={[
					'-mb-px flex items-center gap-2 border-b-2 px-3 py-2 text-sm font-medium transition-colors',
					isActive
						? 'border-accent text-foreground'
						: 'border-transparent text-muted hover:border-line hover:text-foreground'
				].join(' ')}
			>
				<Icon class="size-4" />
				{tab.label}
			</a>
		{/each}
	</nav>

	{@render children()}
</div>
