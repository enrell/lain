<script lang="ts">
	import { tick } from 'svelte';
	import ImageUp from '@lucide/svelte/icons/image-up';
	import Trash2 from '@lucide/svelte/icons/trash-2';
	import { api } from '$lib/api';
	import type { User } from '$lib/api/types';
	import { session } from '$lib/auth/session.svelte';
	import Avatar from '$lib/components/profile/Avatar.svelte';
	import Mascot from '$lib/components/profile/Mascot.svelte';
	import Button from '$lib/components/primitives/Button.svelte';
	import SettingRow from '$lib/components/settings/SettingRow.svelte';
	import SettingsGroup from '$lib/components/settings/SettingsGroup.svelte';
	import LogOut from '@lucide/svelte/icons/log-out';
	import { goto } from '$app/navigation';
	import { SavedFlash } from '$lib/settings/flash.svelte';
	import { MASCOTS, displayName } from '$lib/profile/mascots';
	import { toasts } from '$lib/stores/toasts.svelte';
	import { errorMessage } from '$lib/utilities/errors';
	import { formatDate } from '$lib/utilities/format';

	const MAX_NAME = 40;
	const MAX_BIO = 160;
	const MAX_BYTES = 2 * 1024 * 1024;

	// svelte-ignore state_referenced_locally
	let name = $state(session.user?.profile?.display_name ?? '');
	// svelte-ignore state_referenced_locally
	let bio = $state(session.user?.profile?.bio ?? '');
	let saving = $state(false);
	let uploading = $state(false);
	let fileInput = $state<HTMLInputElement>();
	let grid = $state<HTMLDivElement>();

	const user = $derived(session.user);
	const avatar = $derived(user?.profile?.avatar);
	const dirty = $derived(
		name.trim() !== (user?.profile?.display_name ?? '') || bio.trim() !== (user?.profile?.bio ?? '')
	);
	// A preview of the header with the unsaved name, so the change is visible before saving.
	const preview = $derived<User | null>(
		user ? { ...user, profile: { ...user.profile, avatar: user.profile?.avatar ?? {}, display_name: name } } : null
	);

	const flash = new SavedFlash();

	function apply(u: User): void {
		session.user = u;
	}

	function signOut(): void {
		session.logout();
		void goto('/login');
	}

	async function save(e: SubmitEvent): Promise<void> {
		e.preventDefault();
		saving = true;
		try {
			apply(await api.me.updateProfile({ display_name: name, bio }));
			name = session.user?.profile?.display_name ?? '';
			bio = session.user?.profile?.bio ?? '';
			flash.mark('details');
		} catch (err) {
			toasts.error(errorMessage(err, 'Could not save the profile.'));
		} finally {
			saving = false;
		}
	}

	async function pickMascot(id: string): Promise<void> {
		try {
			apply(await api.me.updateProfile({ mascot: id }));
			flash.mark('avatar');
		} catch (err) {
			toasts.error(errorMessage(err, 'Could not change the avatar.'));
		}
	}

	async function upload(e: Event): Promise<void> {
		const input = e.currentTarget as HTMLInputElement;
		const file = input.files?.[0];
		input.value = '';
		if (!file) return;
		if (file.size > MAX_BYTES) {
			toasts.error('The image must be 2 MB or smaller.');
			return;
		}
		uploading = true;
		try {
			apply(await api.me.uploadAvatar(file));
			flash.mark('avatar');
		} catch (err) {
			toasts.error(errorMessage(err, 'Could not upload the picture.'));
		} finally {
			uploading = false;
		}
	}

	async function remove(): Promise<void> {
		try {
			apply(await api.me.removeAvatar());
			flash.mark('avatar');
		} catch (err) {
			toasts.error(errorMessage(err, 'Could not remove the avatar.'));
		}
	}

	// Mascot picker is a radio group: arrows move and select, like any
	// native radio set; Tab enters on the selected one.
	const selectedIndex = $derived(
		avatar?.kind === 'mascot' ? MASCOTS.findIndex((m) => m.id === avatar.mascot) : -1
	);
	async function onGridKey(e: KeyboardEvent): Promise<void> {
		const cols = window.matchMedia('(min-width: 640px)').matches ? 8 : 4;
		const from = Math.max(0, MASCOTS.findIndex((m) => m.id === (document.activeElement as HTMLElement)?.dataset.mascot));
		const step: Record<string, number> = { ArrowRight: 1, ArrowLeft: -1, ArrowDown: cols, ArrowUp: -cols };
		let to: number | null = null;
		if (e.key in step) to = (from + step[e.key] + MASCOTS.length) % MASCOTS.length;
		else if (e.key === 'Home') to = 0;
		else if (e.key === 'End') to = MASCOTS.length - 1;
		if (to === null) return;
		e.preventDefault();
		await pickMascot(MASCOTS[to].id);
		await tick();
		grid?.querySelector<HTMLElement>(`[data-mascot="${MASCOTS[to].id}"]`)?.focus();
	}
</script>

<svelte:head><title>Profile — Settings — Lain</title></svelte:head>

{#if user}
	<!-- Identity: the live preview of what other people see. -->
	<header class="flex items-center gap-5 pb-8">
		<Avatar user={preview} class="size-20" />
		<div class="min-w-0">
			<p class="truncate text-2xl font-black uppercase tracking-[-0.04em] text-foreground">{displayName(preview)}</p>
			<p class="mt-1 font-mono text-[10px] uppercase tracking-[0.2em] text-muted">
				@{user.username} · {user.role}{#if user.created_at} · since {formatDate(user.created_at)}{/if}
			</p>
			{#if bio.trim()}<p class="mt-2 max-w-xl whitespace-pre-line text-sm text-muted">{bio.trim()}</p>{/if}
		</div>
	</header>

	<form onsubmit={save}>
		<SettingsGroup title="Details">
			<SettingRow id="display-name" label="Display name" hint={`Shown instead of @${user.username}.`} saved={flash.is('details')}>
				<input
					class="h-9 w-full rounded-md border px-2.5 text-sm text-foreground outline-none sm:w-72"
					bind:value={name}
					maxlength={MAX_NAME}
					placeholder={user.username}
					autocomplete="nickname"
					aria-label="Display name"
				/>
				<span class="w-12 text-right font-mono text-[10px] tabular-nums text-muted">{name.length}/{MAX_NAME}</span>
			</SettingRow>
			<SettingRow id="bio" label="Bio" hint="One or two lines.">
				<textarea
					bind:value={bio}
					maxlength={MAX_BIO}
					rows="2"
					aria-label="Bio"
					class="w-full resize-none rounded-md border px-2.5 py-2 text-sm text-foreground outline-none placeholder:text-muted sm:w-72"
					placeholder="A line about you."
					onkeydown={(e) => {
						if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) (e.currentTarget.form as HTMLFormElement).requestSubmit();
					}}
				></textarea>
				<span class="w-12 text-right font-mono text-[10px] tabular-nums text-muted">{bio.length}/{MAX_BIO}</span>
			</SettingRow>
			{#if dirty}
				<div class="flex items-center justify-end gap-3 py-3">
					<span class="font-mono text-[10px] uppercase tracking-[0.14em] text-muted">Ctrl+Enter</span>
					<Button type="submit" size="sm" loading={saving}>Save details</Button>
				</div>
			{/if}
		</SettingsGroup>
	</form>

	<SettingsGroup id="avatar" title="Avatar">
		<SettingRow label="Picture" hint="PNG, JPEG, GIF or WebP · up to 2 MB." saved={flash.is('avatar')}>
			<Button variant="secondary" size="sm" loading={uploading} onclick={() => fileInput?.click()}>
				<ImageUp class="size-4" /> Upload
			</Button>
			{#if avatar?.kind}
				<Button variant="ghost" size="sm" onclick={remove}>
					<Trash2 class="size-4" /> Use initials
				</Button>
			{/if}
			<input bind:this={fileInput} type="file" accept="image/png,image/jpeg,image/gif,image/webp" class="hidden" onchange={upload} />
		</SettingRow>
		<SettingRow label="Mascot" hint="Arrow keys move and pick." stack>
			<!-- svelte-ignore a11y_interactive_supports_focus -->
			<div bind:this={grid} role="radiogroup" aria-label="Mascots" class="grid grid-cols-4 gap-2 sm:grid-cols-8" onkeydown={onGridKey}>
				{#each MASCOTS as m, i (m.id)}
					{@const on = avatar?.kind === 'mascot' && avatar.mascot === m.id}
					<button
						type="button"
						role="radio"
						aria-checked={on}
						aria-label={m.name}
						data-mascot={m.id}
						tabindex={on || (selectedIndex < 0 && i === 0) ? 0 : -1}
						onclick={() => pickMascot(m.id)}
						class="group flex flex-col items-center gap-1.5 rounded-lg py-2 transition-colors hover:bg-surface-hover {on ? 'bg-surface-active' : ''}"
					>
						<span class="block overflow-hidden rounded-full ring-2 transition {on ? 'ring-accent' : 'ring-transparent'}">
							<Mascot id={m.id} class="block size-12" />
						</span>
						<span class="font-mono text-[9px] uppercase tracking-[0.16em] {on ? 'text-accent' : 'text-muted'}">{m.name}</span>
					</button>
				{/each}
			</div>
		</SettingRow>
	</SettingsGroup>

	<SettingsGroup id="account" title="Account">
		<SettingRow label="Username" hint="Used to sign in. Only an admin can change it.">
			<span class="font-mono text-sm text-foreground">@{user.username}</span>
		</SettingRow>
		<SettingRow label="Role">
			<span class="flex items-center gap-1.5 font-mono text-[11px] uppercase tracking-[0.14em] {user.role === 'admin' ? 'text-accent' : 'text-muted'}">
				<span class="size-1.5 rounded-full {user.role === 'admin' ? 'bg-accent' : 'bg-muted'}" aria-hidden="true"></span>{user.role}
			</span>
		</SettingRow>
		<SettingRow label="Session" hint="Signs out of this browser only.">
			<Button variant="ghost" size="sm" onclick={signOut}><LogOut class="size-4" /> Sign out</Button>
		</SettingRow>
	</SettingsGroup>
{/if}
