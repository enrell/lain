<script lang="ts">
	import Avatar from '$lib/components/profile/Avatar.svelte';
	import { displayName } from '$lib/profile/mascots';
	import { onMount } from 'svelte';
	import KeyRound from '@lucide/svelte/icons/key-round';
	import LockIcon from '@lucide/svelte/icons/lock';
	import Trash2 from '@lucide/svelte/icons/trash-2';
	import SlidersHorizontal from '@lucide/svelte/icons/sliders-horizontal';
	import Ellipsis from '@lucide/svelte/icons/ellipsis';
	import Menu, { type MenuEntry } from '$lib/components/primitives/Menu.svelte';
	import UserRound from '@lucide/svelte/icons/user-round';
	import Users from '@lucide/svelte/icons/users';
	import type { User } from '$lib/api/types';
	import { api } from '$lib/api';
	import { session } from '$lib/auth/session.svelte';
	import Badge from '$lib/components/primitives/Badge.svelte';
	import Button from '$lib/components/primitives/Button.svelte';
	import ErrorState from '$lib/components/primitives/ErrorState.svelte';
	import Input from '$lib/components/primitives/Input.svelte';
	import Modal from '$lib/components/primitives/Modal.svelte';
	import Select from '$lib/components/primitives/Select.svelte';
	import Spinner from '$lib/components/primitives/Spinner.svelte';
	import Switch from '$lib/components/primitives/Switch.svelte';
	import SettingsGroup from '$lib/components/settings/SettingsGroup.svelte';
	import { t } from '$lib/i18n';
	import { toasts } from '$lib/stores/toasts.svelte';
	import { errorMessage } from '$lib/utilities/errors';
	import { formatDate } from '$lib/utilities/format';

	let users = $state<User[]>([]);
	let loading = $state(true);
	let error = $state<string | null>(null);
	let rowBusy = $state<string | null>(null);

	let createOpen = $state(false);
	let creating = $state(false);
	let formError = $state<string | null>(null);
	let form = $state({ username: '', password: '', role: 'user' });

	// Deleting asks for the username typed out: it cannot be undone, and a
	// typed name is a confirmation the keyboard reaches as fast as a button.
	let deleteTarget = $state<User | null>(null);
	let deleteOpen = $state(false);
	let deleteTyped = $state('');
	let deleting = $state(false);

	function askDelete(user: User): void {
		deleteTarget = user;
		deleteTyped = '';
		deleteOpen = true;
	}

	async function deleteUser(): Promise<void> {
		const target = deleteTarget;
		if (!target || deleteTyped.trim() !== target.username) return;
		deleting = true;
		try {
			const out = await api.users.remove(target.id);
			users = users.filter((u) => u.id !== target.id);
			deleteOpen = false;
			deleteTarget = null;
			toasts.success(
				out.libraries_reassigned.length > 0
					? t('settings.usersPage.deletedLibraries', { name: target.username, count: out.libraries_reassigned.length })
					: t('settings.usersPage.deleted', { name: target.username })
			);
		} catch (err) {
			toasts.error(errorMessage(err, t('settings.usersPage.deleteFailed')));
		} finally {
			deleting = false;
		}
	}

	let resetTarget = $state<User | null>(null);
	let resetOpen = $state(false);
	let resetPassword = $state('');
	let resetting = $state(false);

	// Per-user playback limits (D-042).
	let playbackTarget = $state<User | null>(null);
	let playbackOpen = $state(false);
	let playbackSaving = $state(false);
	let playbackForm = $state({
		video: true,
		audio: true,
		remux: true,
		maxBitrate: '0',
		maxStreams: '0',
		subtitleMode: ''
	});

	const subtitleModeOptions = $derived([
		{ value: '', label: t('settings.usersPage.subtitleDefault') },
		{ value: 'auto', label: t('settings.transcode.audio.subtitleAuto') },
		{ value: 'extract', label: t('settings.transcode.audio.subtitleExtract') },
		{ value: 'burn', label: t('settings.transcode.audio.subtitleBurn') },
		{ value: 'off', label: t('settings.transcode.audio.subtitleOff') }
	]);

	function openPlaybackLimits(user: User): void {
		playbackTarget = user;
		playbackForm = {
			video: user.playback?.allow_video_transcode ?? true,
			audio: user.playback?.allow_audio_transcode ?? true,
			remux: user.playback?.allow_remux ?? true,
			maxBitrate: String(user.playback?.max_bitrate_kbps ?? 0),
			maxStreams: String(user.playback?.max_streams ?? 0),
			subtitleMode: user.playback?.subtitle_mode ?? ''
		};
		playbackOpen = true;
	}

	async function savePlaybackLimits(): Promise<void> {
		const target = playbackTarget;
		if (!target) return;
		playbackSaving = true;
		try {
			const updated = await api.users.patch(target.id, {
				playback: {
					allow_video_transcode: playbackForm.video,
					allow_audio_transcode: playbackForm.audio,
					allow_remux: playbackForm.remux,
					max_bitrate_kbps: Math.max(0, Number(playbackForm.maxBitrate) || 0),
					max_streams: Math.max(0, Number(playbackForm.maxStreams) || 0),
					subtitle_mode: (playbackForm.subtitleMode || undefined) as
						| 'auto'
						| 'extract'
						| 'burn'
						| 'off'
						| undefined
				}
			});
			users = users.map((u) => (u.id === updated.id ? updated : u));
			playbackOpen = false;
			playbackTarget = null;
			toasts.success(t('settings.usersPage.limitsSaved', { name: target.username }));
		} catch (err) {
			toasts.error(errorMessage(err, t('settings.usersPage.limitsFailed')));
		} finally {
			playbackSaving = false;
		}
	}

	async function load(): Promise<void> {
		loading = true;
		error = null;
		try {
			users = await api.users.list();
		} catch (err) {
			error = errorMessage(err);
		} finally {
			loading = false;
		}
	}

	onMount(() => {
		// Ctrl+K "Add user" lands here with ?add.
		if (new URL(window.location.href).searchParams.has('add')) createOpen = true;
		void load();
	});

	// An account's actions as one labelled menu where the table is two lines (below xl):
	// as bare icons, a sliders glyph on a person's row did not say Playback.
	function rowEntries(user: User): MenuEntry[] {
		return [
			{ label: t('settings.usersPage.playback'), icon: SlidersHorizontal, onSelect: () => openPlaybackLimits(user) },
			{
				label: t('settings.usersPage.resetPassword'),
				icon: KeyRound,
				onSelect: () => {
					resetTarget = user;
					resetOpen = true;
				}
			},
			...(isSelf(user) ? [] : [{ label: t('settings.usersPage.delete'), icon: Trash2, danger: true, separated: true, onSelect: () => askDelete(user) }])
		];
	}

	function isSelf(user: User): boolean {
		return user.id === session.user?.id;
	}

	async function patch(user: User, input: Parameters<typeof api.users.patch>[1], message: string, undo?: () => void) {
		rowBusy = user.id;
		try {
			const updated = await api.users.patch(user.id, input);
			users = users.map((u) => (u.id === updated.id ? updated : u));
			if (undo) toasts.undoable(message, undo);
			else toasts.success(message);
		} catch (err) {
			toasts.error(errorMessage(err, t('settings.usersPage.updateFailed')));
		} finally {
			rowBusy = null;
		}
	}

	async function createUser(event: SubmitEvent): Promise<void> {
		event.preventDefault();
		creating = true;
		formError = null;
		try {
			const user = await api.users.create({
				username: form.username.trim(),
				password: form.password,
				role: form.role as 'admin' | 'user'
			});
			users = [...users, user].sort((a, b) => a.username.localeCompare(b.username));
			createOpen = false;
			form = { username: '', password: '', role: 'user' };
			toasts.success(t('settings.usersPage.created', { name: user.username }));
		} catch (err) {
			formError = errorMessage(err);
		} finally {
			creating = false;
		}
	}

	async function resetUserPassword(): Promise<void> {
		const target = resetTarget;
		if (!target) return;
		resetting = true;
		try {
			await api.users.patch(target.id, { password: resetPassword });
			resetOpen = false;
			resetPassword = '';
			resetTarget = null;
			toasts.success(t('settings.usersPage.passwordReset', { name: target.username }));
		} catch (err) {
			toasts.error(errorMessage(err, t('settings.usersPage.resetFailed')));
		} finally {
			resetting = false;
		}
	}

	const roleOptions = $derived([
		{ value: 'user', label: t('settings.usersPage.roleUser') },
		{ value: 'admin', label: t('settings.usersPage.roleAdmin') }
	]);
</script>

<svelte:head><title>{t('settings.usersPage.pageTitle')}</title></svelte:head>

<SettingsGroup
	id="users"
	title={t('settings.groups.users.title')}
	description={loading ? undefined : t('settings.groups.users.accounts', { count: users.length })}
>
	{#snippet aside()}
		<Button variant="tonal" size="field" onclick={() => (createOpen = true)}>
			<Users class="size-4" /> {t('settings.usersPage.add')}
		</Button>
	{/snippet}
	{#if loading}
		<div class="py-4"><div class="h-12 animate-pulse rounded-md bg-field"></div></div>
	{:else if error}
		<div class="py-4"><ErrorState message={error} retry={() => void load()} /></div>
	{:else if users.length === 0}
		<div class="flex flex-col items-center px-6 py-10 text-center">
			<UserRound class="size-6 text-muted" />
			<p class="mt-3 text-base font-semibold text-foreground">{t('settings.usersPage.none')}</p>
			<p class="mt-1 text-sm text-muted">{t('settings.usersPage.noneBody')}</p>
		</div>
	{:else}
		<!-- One table: who, then role, active, and the per-account actions, in
		     columns that line up from row to row. Your own row cannot change
		     your role, switch you off or delete you: it says so in words
		     instead of drawing controls that look live and do nothing. -->
		<ul class="settings-items">
			{#each users as user (user.id)}
				{@const self = isSelf(user)}
				<li class="group/row flex flex-wrap items-center gap-x-4 gap-y-3 py-3.5 xl:grid xl:grid-cols-[minmax(0,1fr)_8rem_7.5rem_22rem]">
					<!-- Below xl the actions are one labelled menu (⋯) at the end of the
					     name's line and role and access take the second: as a table
					     beside the settings rail at 1024 px the name had no room. -->
					<div class="flex min-w-0 basis-[calc(100%-4rem)] items-center gap-3 xl:basis-auto">
						<Avatar {user} class="size-9 shrink-0" />
						<div class="min-w-0">
							<div class="flex flex-wrap items-center gap-2">
								<p class="truncate text-sm font-medium text-foreground">{displayName(user)}</p>
								{#if user.profile?.display_name}<span class="font-mono text-[11px] text-muted">@{user.username}</span>{/if}
								{#if self}<Badge tone="accent">{t('settings.usersPage.you')}</Badge>{/if}
								{#if user.disabled}<Badge tone="danger">{t('settings.usersPage.disabledBadge')}</Badge>{/if}
								{#if rowBusy === user.id}<Spinner class="size-3.5 text-muted" />{/if}
							</div>
							<p class="mt-0.5 text-xs text-muted">
								{t('settings.usersPage.joined', { role: user.role === 'admin' ? t('settings.usersPage.roleAdmin') : t('settings.usersPage.roleUser'), date: formatDate(user.created_at) })}
							</p>
						</div>
					</div>
					{#if self}
						<p class="flex h-9 items-center gap-1.5 font-mono text-[11px] uppercase tracking-[0.14em] text-muted max-xl:order-2 xl:w-32" title={t('settings.usersPage.selfRole')}>
							<LockIcon class="size-3" aria-hidden="true" />{user.role === 'admin' ? t('settings.usersPage.roleAdmin') : t('settings.usersPage.roleUser')}
						</p>
						<p class="flex h-9 items-center text-[13px] text-muted max-xl:order-2">{t('settings.usersPage.signedInHere')}</p>
					{:else}
						<div class="w-32 max-xl:order-2">
							<Select
								aria-label={t('settings.usersPage.roleFor', { name: user.username })}
								value={user.role}
								options={roleOptions}
								onValueChange={(role) => void patch(user, { role: role as 'admin' | 'user' }, t('settings.usersPage.roleSaved', { name: user.username }))}
							/>
						</div>
						<div class="flex h-9 items-center gap-2 text-sm text-muted max-xl:order-2">
							<!-- Named per row: a list of switches all called "Active" told a screen reader nothing.
							     A mis-click locks someone out at once, so the toast can take it back (Ctrl+Z). -->
							<Switch
								bare
								label={t('settings.groups.users.activeFor', { name: user.username })}
								checked={!user.disabled}
								onCheckedChange={(enabled) =>
									void patch(
										user,
										{ disabled: !enabled },
										t(enabled ? 'settings.groups.users.enabled' : 'settings.groups.users.disabled', { name: user.username }),
										() => void patch(user, { disabled: enabled }, t(enabled ? 'settings.groups.users.disabled' : 'settings.groups.users.enabled', { name: user.username }))
									)}
							/>
							<span>{t('settings.usersPage.active')}</span>
						</div>
					{/if}
					<div class="-me-2 self-start max-xl:order-1 max-xl:ms-auto xl:hidden">
						<Menu label={t('settings.usersPage.actionsFor', { name: user.username })} entries={rowEntries(user)} variant="ghost" size="icon">
							{#snippet icon()}<Ellipsis class="size-5" aria-hidden="true" />{/snippet}
						</Menu>
					</div>
					<!-- A fixed column, filled from its end: every row is its own grid,
					     so an auto column was as wide as that row's buttons and the
					     columns before it drifted from row to row. -->
					<div class="flex flex-nowrap items-center justify-end gap-0.5 max-xl:hidden">
						<Button variant="ghost" size="field" title={t('settings.usersPage.playback')} onclick={() => openPlaybackLimits(user)}>
							<SlidersHorizontal class="size-4" /> {t('settings.usersPage.playback')}
						</Button>
						<Button
							variant="ghost"
							size="field"
							title={t('settings.usersPage.resetPassword')}
							onclick={() => {
								resetTarget = user;
								resetOpen = true;
							}}
						>
							<KeyRound class="size-4" /> {t('settings.usersPage.resetPassword')}
						</Button>
						{#if self}
							<!-- The delete slot kept empty, so Playback and Reset password
							     line up with the rows that have one, at every width. -->
							<span class="block w-[5.5rem] shrink-0" aria-hidden="true"></span>
						{:else}
							<!-- Worded as the actions beside it (a bare bin among words read as
							     a different kind of control); quiet until pointed at, as Remove
							     is on Libraries. -->
							<Button
								variant="ghost"
								size="field"
								class="w-[5.5rem] hover:text-danger! focus-visible:text-danger!"
								aria-label={t('settings.usersPage.deleteFor', { name: user.username })}
								title={t('settings.usersPage.deleteFor', { name: user.username })}
								onclick={() => askDelete(user)}
							>
								<Trash2 class="size-4" /> {t('settings.usersPage.delete')}
							</Button>
						{/if}
					</div>
				</li>
			{/each}
		</ul>
	{/if}
</SettingsGroup>

<!-- Create -->
<Modal bind:open={createOpen} title={t('settings.usersPage.addTitle')} description={t('settings.usersPage.addBody')}>
	<form class="space-y-4" onsubmit={createUser}>
		{#if formError}
			<div
				class="rounded-md border border-danger/30 bg-danger/10 px-3 py-2 text-sm text-danger"
				role="alert"
			>
				{formError}
			</div>
		{/if}
		<Input label={t('settings.usersPage.username')} bind:value={form.username} autocapitalize="none" spellcheck={false} required />
		<Input
			label={t('settings.usersPage.password')}
			type="password"
			bind:value={form.password}
			hint={t('security.tooShort')}
			autocomplete="new-password"
			required
		/>
		<Select label={t('settings.usersPage.role')} bind:value={form.role} options={roleOptions} />
		<div class="flex justify-end gap-2 pt-1">
			<Button type="button" variant="ghost" onclick={() => (createOpen = false)}>{t('common.cancel')}</Button>
			<Button type="submit" loading={creating}>{t('settings.usersPage.create')} <kbd class="ms-1 font-mono text-[10px] opacity-70">Enter</kbd></Button>
		</div>
	</form>
</Modal>

<!-- Reset password -->
<Modal
	bind:open={resetOpen}
	title={t('settings.usersPage.resetTitle')}
	description={t('settings.usersPage.resetBody')}
>
	<form
		class="space-y-4"
		onsubmit={(e) => {
			e.preventDefault();
			void resetUserPassword();
		}}
	>
		<p class="text-sm text-muted">{t('settings.usersPage.resetFor', { name: resetTarget?.username ?? '' })}</p>
		<Input
			label={t('security.new')}
			type="password"
			bind:value={resetPassword}
			hint={t('security.tooShort')}
			autocomplete="new-password"
			required
		/>
		<div class="flex justify-end gap-2 pt-1">
			<Button type="button" variant="ghost" onclick={() => (resetOpen = false)}>{t('common.cancel')}</Button>
			<Button type="submit" loading={resetting} disabled={resetPassword.length < 8}>{t('settings.usersPage.resetPassword')} <kbd class="ms-1 font-mono text-[10px] opacity-70">Enter</kbd></Button>
		</div>
	</form>
</Modal>

<!-- Playback limits (D-042) -->
<Modal
	bind:open={playbackOpen}
	title={t('settings.usersPage.limitsTitle')}
	description={t('settings.usersPage.limitsBody')}
>
	<form
		class="space-y-4"
		onsubmit={(e) => {
			e.preventDefault();
			void savePlaybackLimits();
		}}
	>
		<p class="text-sm text-muted">{t('settings.usersPage.limitsFor', { name: playbackTarget?.username ?? '' })}</p>
		<Switch
			label={t('settings.usersPage.allowVideo')}
			description={t('settings.usersPage.allowVideoHint')}
			checked={playbackForm.video}
			onCheckedChange={(checked) => (playbackForm.video = checked)}
		/>
		<Switch
			label={t('settings.usersPage.allowAudio')}
			description={t('settings.usersPage.allowAudioHint')}
			checked={playbackForm.audio}
			onCheckedChange={(checked) => (playbackForm.audio = checked)}
		/>
		<Switch
			label={t('settings.usersPage.allowRemux')}
			description={t('settings.usersPage.allowRemuxHint')}
			checked={playbackForm.remux}
			onCheckedChange={(checked) => (playbackForm.remux = checked)}
		/>
		<Input
			label={t('settings.usersPage.maxBitrate')}
			type="number"
			min="0"
			bind:value={playbackForm.maxBitrate}
		/>
		<Input
			label={t('settings.usersPage.maxStreams')}
			type="number"
			min="0"
			max="32"
			bind:value={playbackForm.maxStreams}
			hint={t('settings.usersPage.maxStreamsHint')}
		/>
		<Select
			label={t('settings.usersPage.subtitleMode')}
			value={playbackForm.subtitleMode}
			options={subtitleModeOptions}
			onValueChange={(value) => (playbackForm.subtitleMode = value)}
		/>
		<div class="flex justify-end gap-2 pt-1">
			<Button type="button" variant="ghost" onclick={() => (playbackOpen = false)}>{t('common.cancel')}</Button>
			<Button type="submit" loading={playbackSaving}>{t('settings.usersPage.saveLimits')} <kbd class="ms-1 font-mono text-[10px] opacity-70">Enter</kbd></Button>
		</div>
	</form>
</Modal>

<!-- Delete: names what goes with the account, and asks for its name. -->
<Modal
	bind:open={deleteOpen}
	title={t('settings.usersPage.deleteTitle', { name: deleteTarget?.username ?? '' })}
	description={t('settings.usersPage.deleteBody')}
>
	<form
		class="space-y-4"
		onsubmit={(e) => {
			e.preventDefault();
			void deleteUser();
		}}
	>
		<p class="font-mono text-[10px] uppercase tracking-[0.16em] text-muted">{t('settings.usersPage.goesWith')}</p>
		<ul class="-mt-2 list-disc space-y-1 ps-5 text-sm text-muted marker:text-muted/60">
			<li>{t('settings.usersPage.loseProgress')}</li>
			<li>{t('settings.usersPage.loseLists')}</li>
			<li>{t('settings.usersPage.loseSocial')}</li>
		</ul>
		<p class="text-xs text-muted">{t('settings.usersPage.keeps')}</p>
		<Input
			label={t('settings.usersPage.typeName', { name: deleteTarget?.username ?? '' })}
			bind:value={deleteTyped}
			autocapitalize="none"
			autocomplete="off"
			spellcheck={false}
		/>
		<div class="flex justify-end gap-2 pt-1">
			<Button type="button" variant="ghost" onclick={() => (deleteOpen = false)}>{t('common.cancel')} <kbd class="ms-1 font-mono text-[10px] text-muted">Esc</kbd></Button>
			<Button type="submit" variant="danger" loading={deleting} disabled={deleteTyped.trim() !== deleteTarget?.username}>
				{t('settings.usersPage.deleteConfirm', { name: deleteTarget?.username ?? '' })} <kbd class="ms-1 font-mono text-[10px] opacity-70">Enter</kbd>
			</Button>
		</div>
	</form>
</Modal>
