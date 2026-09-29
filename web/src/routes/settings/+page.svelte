<script lang="ts">
	import { onMount } from 'svelte';
	import LogOut from '@lucide/svelte/icons/log-out';
	import RefreshCw from '@lucide/svelte/icons/refresh-cw';
	import ShieldCheck from '@lucide/svelte/icons/shield-check';
	import Unlink from '@lucide/svelte/icons/unlink';
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import { api, type LinkedAccountView } from '$lib/api';
	import { session } from '$lib/auth/session.svelte';
	import Badge from '$lib/components/primitives/Badge.svelte';
	import Button from '$lib/components/primitives/Button.svelte';
	import Input from '$lib/components/primitives/Input.svelte';
	import Spinner from '$lib/components/primitives/Spinner.svelte';
	import { toasts } from '$lib/stores/toasts.svelte';
	import { errorMessage } from '$lib/utilities/errors';
	import { formatDate, formatRelative } from '$lib/utilities/format';
	import { loadPreferredPlayer, savePreferredPlayer, type PreferredPlayer } from '$lib/player/external-player';

	let preferredPlayer = $state<PreferredPlayer>('browser');
	let preferredLanguage = $state('');
	let savingLanguage = $state(false);
	onMount(() => {
		if (session.user) {
			preferredPlayer = loadPreferredPlayer(session.user.id);
			preferredLanguage = session.user.preferred_language ?? '';
		}
	});
	function changePlayer(value: string): void {
		if (!session.user || (value !== 'browser' && value !== 'mpv' && value !== 'vlc')) return;
		preferredPlayer = value;
		savePreferredPlayer(session.user.id, value);
	}
	async function saveLanguage(): Promise<void> {
		const language = preferredLanguage.trim().toLowerCase();
		if (language && !/^[a-z]{3}$/.test(language)) {
			toasts.error('Use a three-letter ISO 639-2 code, such as por, eng or jpn.');
			return;
		}
		savingLanguage = true;
		try {
			session.user = await api.me.setPreferredLanguage(language);
			preferredLanguage = language;
			toasts.success('Preferred playback language saved to your account.');
		} catch (err) {
			toasts.error(errorMessage(err, 'Could not save the language preference.'));
		} finally {
			savingLanguage = false;
		}
	}

	let oldPassword = $state('');
	let newPassword = $state('');
	let confirm = $state('');
	let fieldErrors = $state<{ new?: string; confirm?: string }>({});
	let error = $state<string | null>(null);
	let busy = $state(false);

	async function submit(event: SubmitEvent): Promise<void> {
		event.preventDefault();
		if (busy) return;
		error = null;
		const next: typeof fieldErrors = {};
		if (newPassword.length < 8) next.new = 'At least 8 characters.';
		if (confirm !== newPassword) next.confirm = 'Passwords do not match.';
		fieldErrors = next;
		if (Object.keys(next).length > 0) return;

		busy = true;
		try {
			await api.me.changePassword(oldPassword, newPassword);
			// Rotating the password bumps the token version: mint a fresh
			// session with the new password instead of getting 401'd.
			await session.login(session.user?.username ?? '', newPassword);
			oldPassword = '';
			newPassword = '';
			confirm = '';
			toasts.success('Password changed. Other sessions were signed out.');
		} catch (err) {
			error = errorMessage(err, 'Could not change the password.');
		} finally {
			busy = false;
		}
	}

	function signOut(): void {
		session.logout();
		void goto('/login');
	}

	// --- Connected list accounts (D-079): AniList today; each entry is a
	// platform that offers a supported link flow.
	const platforms: { id: string; name: string }[] = [{ id: 'anilist', name: 'AniList' }];
	let links = $state<LinkedAccountView[]>([]);
	let linksReady = $state(false);
	let linkBusy = $state('');
	// Pin flow: the official Lain AniList app lets users link without any
	// admin setup — authorize on AniList, paste the token the pin page
	// shows (D-080). Hidden until the server reports a pin client.
	let pinUrl = $state<string | null>(null);
	let pinOpen = $state(false);
	let pinCode = $state('');
	let pinBusy = $state(false);
	let pinErr = $state('');

	const linkFor = (platform: string) => links.find((l) => l.platform === platform);
	const linkNeedsReconnect = (l: LinkedAccountView) =>
		l.token_expired === true || l.last_sync_error === 'token-invalid' || l.last_sync_error === 'token-expired';

	async function loadLinks(): Promise<void> {
		try {
			const res = await api.links.list();
			links = res.links;
			try {
				pinUrl = (await api.links.pin('anilist')).url;
			} catch {
				pinUrl = null;
			}
		} catch (err) {
			toasts.error(errorMessage(err, 'Could not load connected accounts.'));
		} finally {
			linksReady = true;
		}
	}

	async function connect(platform: string): Promise<void> {
		linkBusy = platform;
		try {
			const res = await api.links.authorize(platform);
			// Full-page navigation: the platform consent screen is remote.
			window.location.href = res.url;
		} catch (err) {
			const configured =
				err instanceof Error && 'code' in err && (err as { code?: string }).code === 'not-configured';
			if (configured && pinUrl) {
				pinOpen = true;
				toasts.error('Redirect sign-in needs the administrator app — use the code flow below.');
			} else {
				toasts.error(
					configured
						? `${platform} is not configured on this server — ask the administrator.`
						: errorMessage(err, `Could not start the ${platform} connection.`)
				);
			}
			linkBusy = '';
		}
	}

	function openPin(): void {
		if (pinUrl) window.open(pinUrl, '_blank', 'noopener');
	}

	async function submitPin(platform: string): Promise<void> {
		const code = pinCode.trim();
		if (!code) {
			pinErr = 'Paste the code the AniList page shows.';
			return;
		}
		pinBusy = true;
		pinErr = '';
		try {
			await api.links.code(platform, code);
			toasts.success(`${platform} connected — importing your list.`);
			pinOpen = false;
			pinCode = '';
			await loadLinks();
		} catch (err) {
			const code = err instanceof Error && 'code' in err ? (err as { code?: string }).code : '';
			pinErr =
				code === 'invalid-grant'
					? 'AniList rejected that code — authorize again and copy the whole code.'
					: errorMessage(err, 'Could not link the account.');
		} finally {
			pinBusy = false;
		}
	}

	async function syncNow(platform: string): Promise<void> {
		linkBusy = platform;
		try {
			const res = await api.links.sync(platform);
			toasts.success(`Synced: ${res.stats.upserted} entries, ${res.stats.removed} removed.`);
			await loadLinks();
		} catch (err) {
			toasts.error(errorMessage(err, 'Sync failed.'));
		} finally {
			linkBusy = '';
		}
	}

	async function toggleScrobble(link: LinkedAccountView): Promise<void> {
		linkBusy = link.platform;
		try {
			await api.links.setScrobble(link.platform, !link.scrobble);
			await loadLinks();
		} catch (err) {
			toasts.error(errorMessage(err, 'Could not change the setting.'));
		} finally {
			linkBusy = '';
		}
	}

	async function disconnect(platform: string): Promise<void> {
		linkBusy = platform;
		try {
			await api.links.unlink(platform);
			toasts.success(`${platform} disconnected; imported entries removed.`);
			await loadLinks();
		} catch (err) {
			toasts.error(errorMessage(err, 'Could not disconnect.'));
		} finally {
			linkBusy = '';
		}
	}

	onMount(() => {
		// The OAuth callback lands back here with a result flag.
		const linked = page.url.searchParams.get('linked');
		const linkError = page.url.searchParams.get('link_error');
		if (linked) toasts.success(`${linked} connected — importing your list.`);
		if (linkError) toasts.error(`The connection did not complete (${linkError}).`);
		if (linked || linkError) {
			const url = new URL(page.url);
			url.searchParams.delete('linked');
			url.searchParams.delete('link_error');
			void goto(url.pathname + url.search, { replaceState: true });
		}
		void loadLinks();
	});
</script>

<svelte:head><title>Account — Settings — Lain</title></svelte:head>

<div class="grid gap-6 lg:grid-cols-2">
	<section class="rounded-card border border-line bg-surface/60 p-5">
		<h2 class="text-base font-semibold text-foreground">Default player</h2>
		<p class="mt-1 text-sm text-muted">Choose what Play opens in this browser. mpv and VLC launch locally when the server shares this machine; elsewhere they go through the CLI handler.</p>
		<label class="mt-4 block text-sm font-medium text-foreground" for="preferred-player">Player</label>
		<select id="preferred-player" class="mt-2 w-full rounded-md border border-line bg-background p-2 text-foreground" value={preferredPlayer} onchange={(e) => changePlayer(e.currentTarget.value)}>
			<option value="browser">Web player</option>
			<option value="mpv">mpv</option>
			<option value="vlc">VLC</option>
		</select>
		<p class="mt-2 text-xs text-muted">When this browser and the server are on the same computer, mpv and VLC open locally — no setup needed. On a different computer, run <code>lain login</code> and <code>lain install-player-handler</code> there.</p>
	</section>
	<section class="rounded-card border border-line bg-surface/60 p-5">
		<h2 class="text-base font-semibold text-foreground">Preferred playback language</h2>
		<p class="mt-1 text-sm text-muted">When audio is available in this language, subtitles stay off. Otherwise Lain looks for subtitles in this language. You can still change tracks in the player.</p>
		<label class="mt-4 block text-sm font-medium text-foreground" for="preferred-language">Language code</label>
		<input id="preferred-language" class="mt-2 w-full rounded-md border border-line bg-background p-2 text-foreground sm:max-w-xs" type="text" list="common-languages" maxlength="3" autocomplete="off" spellcheck="false" placeholder="Choose or enter a code" bind:value={preferredLanguage} />
		<datalist id="common-languages"><option value="por">Portuguese</option><option value="eng">English</option><option value="jpn">Japanese</option><option value="spa">Spanish</option><option value="fra">French</option><option value="deu">German</option><option value="ita">Italian</option><option value="kor">Korean</option><option value="zho">Chinese</option></datalist>
		<p class="mt-2 text-xs text-muted">Use a three-letter ISO 639-2 code. Leave blank for each player's default behavior. This preference follows your account across web and CLI.</p>
		<Button class="mt-4" loading={savingLanguage} onclick={() => void saveLanguage()}>Save language</Button>
	</section>
	<section class="rounded-card border border-line bg-surface/60 p-5">
		<h2 class="text-base font-semibold text-foreground">Profile</h2>
		<dl class="mt-4 space-y-3 text-sm">
			<div class="flex items-center justify-between gap-4">
				<dt class="text-muted">Username</dt>
				<dd class="font-medium text-foreground">{session.user?.username}</dd>
			</div>
			<div class="flex items-center justify-between gap-4">
				<dt class="text-muted">Role</dt>
				<dd>
					<Badge tone={session.isAdmin ? 'accent' : 'neutral'}>
						{#if session.isAdmin}<ShieldCheck class="size-3" />{/if}
						{session.user?.role}
					</Badge>
				</dd>
			</div>
			{#if session.user?.created_at}
				<div class="flex items-center justify-between gap-4">
					<dt class="text-muted">Member since</dt>
					<dd class="text-foreground">{formatDate(session.user.created_at)}</dd>
				</div>
			{/if}
		</dl>
		<Button variant="secondary" class="mt-5" onclick={signOut}>
			<LogOut class="size-4" /> Sign out
		</Button>
	</section>

	<section class="rounded-card border border-line bg-surface/60 p-5 lg:col-span-2">
		<h2 class="text-base font-semibold text-foreground">Connected accounts</h2>
		<p class="mt-1 text-sm text-muted">
			Link an external list account to import it into your Lain list. The remote platform
			stays the source of truth for what it imported; disconnecting removes its entries.
		</p>
		{#if !linksReady}
			<div class="mt-6 flex justify-center"><Spinner class="size-5 text-muted" /></div>
		{:else}
			<ul class="mt-4 space-y-3">
				{#each platforms as p (p.id)}
					{@const link = linkFor(p.id)}
					<li class="flex flex-wrap items-center justify-between gap-3 rounded-lg border border-line bg-background/40 px-4 py-3">
						{#if link}
							<div class="min-w-0">
								<p class="flex items-center gap-2 text-sm font-medium text-foreground">
									{p.name}
									{#if linkNeedsReconnect(link)}
										<Badge tone="danger">Reconnect needed</Badge>
									{:else}
										<Badge tone="accent">Connected</Badge>
									{/if}
								</p>
								<p class="mt-0.5 truncate text-xs text-muted">
									{link.remote_username || link.remote_user_id}
									· {link.entry_count} {link.entry_count === 1 ? 'entry' : 'entries'}
									{#if link.last_sync_at}
										· synced {formatRelative(link.last_sync_at)}
									{:else if link.last_sync_error}
										· sync failed ({link.last_sync_error})
									{/if}
								</p>
								<button
									type="button"
									class="mt-1 font-mono text-[10px] uppercase tracking-[0.14em] text-muted hover:text-foreground"
									aria-pressed={link.scrobble}
									disabled={linkBusy === p.id}
									onclick={() => void toggleScrobble(link)}
								>
									Update {p.name} when I finish an episode: {link.scrobble ? 'on' : 'off'}
								</button>
							</div>
							<div class="flex items-center gap-2">
								{#if linkNeedsReconnect(link)}
									<Button
										variant="secondary"
										class="!px-3 !py-1.5 text-xs"
										loading={linkBusy === p.id}
										onclick={() => void connect(p.id)}
									>
										Reconnect
									</Button>
								{:else}
									<Button
										variant="secondary"
										class="!px-3 !py-1.5 text-xs"
										loading={linkBusy === p.id}
										onclick={() => void syncNow(p.id)}
									>
										<RefreshCw class="size-3.5" /> Sync now
									</Button>
								{/if}
								<Button
									variant="secondary"
									class="!px-3 !py-1.5 text-xs"
									loading={linkBusy === p.id}
									onclick={() => void disconnect(p.id)}
								>
									<Unlink class="size-3.5" /> Disconnect
								</Button>
							</div>
						{:else}
							<div class="w-full">
								<div class="flex flex-wrap items-center justify-between gap-3">
									<div class="min-w-0">
										<p class="text-sm font-medium text-foreground">{p.name}</p>
										<p class="mt-0.5 text-xs text-muted">Not connected.</p>
									</div>
									<div class="flex items-center gap-2">
										<Button
											variant="secondary"
											class="!px-3 !py-1.5 text-xs"
											loading={linkBusy === p.id}
											onclick={() => void connect(p.id)}
										>
											Connect
										</Button>
										{#if pinUrl && !pinOpen}
											<button
												type="button"
												class="cursor-pointer text-xs text-muted underline decoration-dotted underline-offset-2 hover:text-foreground"
												onclick={() => (pinOpen = true)}
											>
												Use a code instead
											</button>
										{/if}
									</div>
								</div>
								{#if pinOpen && pinUrl}
									<div class="mt-3 rounded-md border border-line bg-surface/60 p-3">
										<p class="text-xs text-muted">
											Authorize Lain on AniList, then paste the code the page shows:
										</p>
										<div class="mt-2 flex flex-wrap items-center gap-2">
											<Button variant="secondary" class="!px-3 !py-1.5 text-xs" onclick={openPin}>
												Open AniList
											</Button>
											<div class="min-w-40 flex-1">
												<Input
													type="password"
													placeholder="Paste the code"
													bind:value={pinCode}
													error={pinErr || null}
												/>
											</div>
											<Button
												class="!px-3 !py-1.5 text-xs"
												loading={pinBusy}
												onclick={() => void submitPin(p.id)}
											>
												Link
											</Button>
										</div>
									</div>
								{/if}
							</div>
						{/if}
					</li>
				{/each}
			</ul>
		{/if}
	</section>

	<section class="rounded-card border border-line bg-surface/60 p-5">
		<h2 class="text-base font-semibold text-foreground">Change password</h2>
		<p class="mt-1 text-sm text-muted">
			Changing the password signs out every other session immediately.
		</p>
		<form class="mt-4 space-y-4" onsubmit={submit}>
			{#if error}
				<div
					class="rounded-md border border-danger/30 bg-danger/10 px-3 py-2 text-sm text-danger"
					role="alert"
				>
					{error}
				</div>
			{/if}
			<Input
				label="Current password"
				type="password"
				bind:value={oldPassword}
				autocomplete="current-password"
				required
			/>
			<Input
				label="New password"
				type="password"
				bind:value={newPassword}
				error={fieldErrors.new ?? null}
				hint="At least 8 characters."
				autocomplete="new-password"
				required
			/>
			<Input
				label="Confirm new password"
				type="password"
				bind:value={confirm}
				error={fieldErrors.confirm ?? null}
				autocomplete="new-password"
				required
			/>
			<Button type="submit" loading={busy}>Update password</Button>
		</form>
	</section>
</div>
