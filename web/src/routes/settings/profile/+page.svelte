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
	import Check from '@lucide/svelte/icons/check';
	import { beforeNavigate, goto } from '$app/navigation';
	import Modal from '$lib/components/primitives/Modal.svelte';
	import { SavedFlash } from '$lib/settings/flash.svelte';
	import { i18n, localeInfo, t } from '$lib/i18n';
	import { MASCOTS, displayName } from '$lib/profile/mascots';
	import { toasts } from '$lib/stores/toasts.svelte';
	import { errorMessage } from '$lib/utilities/errors';
	import { formatDate } from '$lib/utilities/format';

	const MAX_NAME = 40;
	const MAX_BIO = 160;
	const MAX_BYTES = 2 * 1024 * 1024;

	// svelte-ignore state_referenced_locally
	// The field starts with the name in use (the username when none was set), not with a placeholder that reads as empty.
	let name = $state(session.user?.profile?.display_name || session.user?.username || '');
	// svelte-ignore state_referenced_locally
	let bio = $state(session.user?.profile?.bio ?? '');
	let saving = $state(false);
	let uploading = $state(false);
	let fileInput = $state<HTMLInputElement>();
	let grid = $state<HTMLDivElement>();

	const user = $derived(session.user);
	const avatar = $derived(user?.profile?.avatar);
	const dirty = $derived(
		name.trim() !== (user?.profile?.display_name || user?.username || '') || bio.trim() !== (user?.profile?.bio ?? '')
	);
	// A preview of the header with the unsaved name, so the change is visible before saving.
	const preview = $derived<User | null>(
		user ? { ...user, profile: { ...user.profile, avatar: user.profile?.avatar ?? {}, display_name: name } } : null
	);

	const flash = new SavedFlash();

	function apply(u: User): void {
		session.user = u;
	}

	async function changeLocale(choice: string): Promise<void> {
		await i18n.setLocale(choice);
		flash.mark('locale');
	}

	function signOut(): void {
		signingOut = true;
		session.logout();
		void goto('/login');
	}

	async function save(e: SubmitEvent): Promise<void> {
		e.preventDefault();
		await saveDetails();
	}

	function discardDetails(): void {
		name = user?.profile?.display_name || user?.username || '';
		bio = user?.profile?.bio ?? '';
	}

	async function saveDetails(): Promise<boolean> {
		saving = true;
		try {
			apply(await api.me.updateProfile({ display_name: name, bio }));
			name = session.user?.profile?.display_name || session.user?.username || '';
			bio = session.user?.profile?.bio ?? '';
			flash.mark('details');
			return true;
		} catch (err) {
			toasts.error(errorMessage(err, t('settings.profilePage.saveFailed')));
			return false;
		} finally {
			saving = false;
		}
	}

	/*
	 * The name and bio wait for Save; leaving with them changed asks first
	 * (Save and leave, Leave without saving, Stay), as staged server settings
	 * do, instead of dropping what was typed. Closing the tab gets the
	 * browser's own question.
	 */
	let leaveTo = $state<URL | null>(null);
	// A held Back or Forward is replayed as one, so the history stays as it was.
	let leaveDelta = 0;
	let asking = $state(false);
	// Lets the next navigation through: the one this page starts itself.
	let passing = false;
	// Signing out leaves whatever was typed, through however many redirects.
	let signingOut = false;
	beforeNavigate((nav) => {
		if (passing) {
			passing = false;
			return;
		}
		if (!dirty || signingOut) return;
		nav.cancel();
		if (nav.willUnload || !nav.to) return;
		leaveTo = nav.to.url;
		leaveDelta = nav.type === 'popstate' ? nav.delta : 0;
		asking = true;
	});
	function leave(): void {
		const to = leaveTo;
		const delta = leaveDelta;
		asking = false;
		leaveTo = null;
		leaveDelta = 0;
		if (!to) return;
		passing = true;
		if (delta) history.go(delta);
		else void goto(to.pathname + to.search + to.hash);
	}
	async function saveAndLeave(): Promise<void> {
		asking = false;
		if (await saveDetails()) leave();
	}

	async function pickMascot(id: string): Promise<void> {
		try {
			apply(await api.me.updateProfile({ mascot: id }));
			flash.mark('avatar');
		} catch (err) {
			toasts.error(errorMessage(err, t('settings.profilePage.avatarFailed')));
		}
	}

	async function upload(e: Event): Promise<void> {
		const input = e.currentTarget as HTMLInputElement;
		const file = input.files?.[0];
		input.value = '';
		if (!file) return;
		if (file.size > MAX_BYTES) {
			toasts.error(t('settings.profilePage.tooBig'));
			return;
		}
		uploading = true;
		try {
			apply(await api.me.uploadAvatar(file));
			flash.mark('avatar');
		} catch (err) {
			toasts.error(errorMessage(err, t('settings.profilePage.uploadFailed')));
		} finally {
			uploading = false;
		}
	}

	async function remove(): Promise<void> {
		try {
			apply(await api.me.removeAvatar());
			flash.mark('avatar');
		} catch (err) {
			toasts.error(errorMessage(err, t('settings.profilePage.removeFailed')));
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

<svelte:head><title>{t('settings.profilePage.pageTitle')}</title></svelte:head>

{#if user}
	<!-- Identity: the live preview of what other people see, in a panel of
	     its own above the groups that change it. -->
	<div class="mb-10 flex items-center gap-5 rounded-card bg-panel p-5">
		<Avatar user={preview} class="size-16 shrink-0" />
		<div class="min-w-0">
			<!-- As typed: the card is the preview of what others see, and capitals here read "ANA" over a field that says "ana". -->
			<p class="truncate text-2xl font-black tracking-[-0.03em] text-foreground">{displayName(preview)}</p>
			<p class="mt-1 font-mono text-[10px] uppercase tracking-[0.2em] text-muted">
				<!-- One joined string: a block's leading space was trimmed into "ADMIN· SINCE". -->
				{[`@${user.username}`, user.role, user.created_at ? t('settings.since', { date: formatDate(user.created_at) }) : ''].filter(Boolean).join(' · ')}
			</p>
			{#if bio.trim()}<p class="mt-2 max-w-xl whitespace-pre-line text-sm text-muted">{bio.trim()}</p>{/if}
		</div>
	</div>

	<!-- Ctrl+Enter saves from either field, as the Save button says: it
	     was the bio's alone, and in the name field it did nothing. -->
	<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
	<form
		onsubmit={save}
		onkeydown={(e) => {
			if (e.key !== 'Enter' || !(e.ctrlKey || e.metaKey)) return;
			e.preventDefault();
			e.currentTarget.requestSubmit();
		}}
	>
		<SettingsGroup title={t('settings.profilePage.details')} description={t('settings.groups.profile.details')}>
			<SettingRow id="display-name" label={t('settings.entry.displayName')} hint={t('settings.profilePage.nameHint', { username: user.username })} saved={flash.is('details')}>
				<!-- The count sits inside its field's end: beside it, it floated in a column of its
				     own and the field stopped short of every other control's right edge. -->
				<div class="relative w-full">
					<input
						class="h-9 w-full rounded-md border px-3 pe-12 text-sm text-foreground outline-none"
						bind:value={name}
						maxlength={MAX_NAME}
						placeholder={user.username}
						autocomplete="nickname"
						aria-label={t('settings.entry.displayName')}
					/>
					<!-- The count appears with the first character: beside a placeholder, "0/40" read as a broken counter. -->
					<span class="pointer-events-none absolute inset-y-0 end-2.5 flex items-center font-mono text-[10px] tabular-nums text-muted" class:invisible={name.length === 0}>{name.length}/{MAX_NAME}</span>
				</div>
			</SettingRow>
			<SettingRow id="bio" label={t('settings.entry.bio')} hint={t('settings.profilePage.bioHint')} saved={flash.is('details')}>
				<div class="relative w-full">
					<textarea
						bind:value={bio}
						maxlength={MAX_BIO}
						rows="2"
						aria-label={t('settings.entry.bio')}
						class="block w-full resize-none rounded-md border px-3 pb-5 pt-2 text-sm text-foreground outline-none placeholder:text-muted"
						placeholder={t('settings.profilePage.bioPlaceholder')}
					></textarea>
					<span class="pointer-events-none absolute bottom-1.5 end-2.5 font-mono text-[10px] tabular-nums text-muted" class:invisible={bio.length === 0}>{bio.length}/{MAX_BIO}</span>
				</div>
			</SettingRow>
			<!-- Always says where the name and bio stand: saved (what others see
			     now), or changed and waiting for Save. A row that appeared only
			     once something changed left a clean form saying nothing. -->
			<div class="flex min-h-14 flex-wrap items-center justify-between gap-3 py-3" aria-live="polite">
				{#if dirty}
					<p class="flex items-center gap-2 text-[13px] text-foreground">
						<span class="size-1.5 rounded-full bg-accent" aria-hidden="true"></span>{t('settings.profilePage.unsaved')}
					</p>
					<div class="flex items-center gap-2">
						<Button type="button" variant="ghost" size="field" onclick={discardDetails}>{t('settings.profilePage.discard')}</Button>
						<Button type="submit" size="field" loading={saving}>
							{t('settings.profilePage.save')} <kbd class="ms-1 font-mono text-[10px] opacity-70">Ctrl Enter</kbd>
						</Button>
					</div>
				{:else}
					<p class="flex items-center gap-1.5 font-mono text-[10px] uppercase tracking-[0.14em] text-muted">
						<Check class="size-3" aria-hidden="true" /> {t('settings.profilePage.savedState')}
					</p>
				{/if}
			</div>
		</SettingsGroup>
	</form>

	<SettingsGroup id="avatar" title={t('settings.entry.avatar')} description={t('settings.groups.profile.avatar')}>
		<SettingRow label={t('settings.profilePage.picture')} hint={t('settings.profilePage.pictureHint')} saved={flash.is('avatar')}>
			<Button variant="tonal" size="field" loading={uploading} onclick={() => fileInput?.click()}>
				<ImageUp class="size-4" /> {t('settings.profilePage.upload')}
			</Button>
			{#if avatar?.kind}
				<Button variant="ghost" size="field" onclick={remove}>
					<Trash2 class="size-4" /> {t('settings.profilePage.initials')}
				</Button>
			{/if}
			<input bind:this={fileInput} type="file" accept="image/png,image/jpeg,image/gif,image/webp" class="hidden" onchange={upload} />
		</SettingRow>
		<SettingRow label={t('settings.profilePage.mascot')} hint={t('settings.profilePage.mascotHint')} stack>
			<!-- svelte-ignore a11y_interactive_supports_focus -->
			<div bind:this={grid} role="radiogroup" aria-label={t('settings.profilePage.mascots')} class="grid grid-cols-4 gap-2 sm:grid-cols-8" onkeydown={onGridKey}>
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

	<SettingsGroup id="interface" title={t('settings.interface.group')} description={t('settings.groups.profile.interface')}>
		<SettingRow label={t('settings.interface.language')} hint={t('settings.interface.languageHint')} saved={flash.is('locale')}>
			<select
				class="h-10 w-full rounded-md border px-3 text-sm text-foreground outline-none"
				aria-label={t('settings.interface.language')}
				value={i18n.choice}
				onchange={(e) => void changeLocale(e.currentTarget.value)}
			>
				<option value="">{t('settings.interface.system', { name: localeInfo(i18n.systemLocale).name })}</option>
				{#each i18n.available as locale (locale.code)}
					<option value={locale.code}>{locale.name}</option>
				{/each}
			</select>
		</SettingRow>
	</SettingsGroup>

	<SettingsGroup id="account" title={t('settings.profilePage.account')} description={t('settings.groups.profile.account')}>
		<SettingRow label={t('settings.usersPage.username')} hint={t('settings.profilePage.usernameHint')}>
			<span class="font-mono text-sm text-foreground">@{user.username}</span>
		</SettingRow>
		<SettingRow label={t('settings.usersPage.role')}>
			<span class="flex items-center gap-1.5 font-mono text-[11px] uppercase tracking-[0.14em] {user.role === 'admin' ? 'text-accent' : 'text-muted'}">
				<span class="size-1.5 rounded-full {user.role === 'admin' ? 'bg-accent' : 'bg-muted'}" aria-hidden="true"></span>{user.role === 'admin' ? t('settings.usersPage.roleAdmin') : t('settings.usersPage.roleUser')}
			</span>
		</SettingRow>
		<SettingRow label={t('settings.profilePage.session')} hint={t('settings.profilePage.sessionHint')}>
			<Button variant="tonal" size="field" onclick={signOut}><LogOut class="size-4" /> {t('common.signOut')}</Button>
		</SettingRow>
	</SettingsGroup>
{/if}

<Modal bind:open={asking} title={t('settings.profileLeave.title')} description={t('settings.profileLeave.body')}>
	{#snippet footer()}
		<Button variant="ghost" onclick={() => (asking = false)}>{t('settings.staged.stay')} <kbd class="ml-1 font-mono text-[10px] text-muted">Esc</kbd></Button>
		<Button variant="secondary" onclick={leave}>{t('settings.profileLeave.discard')}</Button>
		<Button loading={saving} onclick={() => void saveAndLeave()}>{t('settings.profileLeave.save')} <kbd class="ml-1 font-mono text-[10px] opacity-70">Enter</kbd></Button>
	{/snippet}
</Modal>
