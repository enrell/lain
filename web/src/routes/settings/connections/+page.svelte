<script lang="ts">
	import { onMount } from 'svelte';
	import RefreshCw from '@lucide/svelte/icons/refresh-cw';
	import Unlink from '@lucide/svelte/icons/unlink';
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import { api, type LinkedAccountView } from '$lib/api';
	import Button from '$lib/components/primitives/Button.svelte';
	import Input from '$lib/components/primitives/Input.svelte';
	import Spinner from '$lib/components/primitives/Spinner.svelte';
	import Switch from '$lib/components/primitives/Switch.svelte';
	import SettingRow from '$lib/components/settings/SettingRow.svelte';
	import SettingsGroup from '$lib/components/settings/SettingsGroup.svelte';
	import { toasts } from '$lib/stores/toasts.svelte';
	import { errorMessage } from '$lib/utilities/errors';
	import { formatRelative } from '$lib/utilities/format';

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

<svelte:head><title>Connections — Settings — Lain</title></svelte:head>

<SettingsGroup title="Lists">
	{#if !linksReady}
		<div class="flex justify-center py-8"><Spinner class="size-5 text-muted" /></div>
	{:else}
		{#each platforms as p (p.id)}
			{@const link = linkFor(p.id)}
			<SettingRow
				id={p.id}
				label={p.name}
				hint={link
					? `${link.remote_username || link.remote_user_id} · ${link.entry_count} ${link.entry_count === 1 ? 'entry' : 'entries'}${link.last_sync_at ? ` · synced ${formatRelative(link.last_sync_at)}` : link.last_sync_error ? ` · sync failed (${link.last_sync_error})` : ''}`
					: 'Import your list. The remote stays the source of truth; disconnecting removes its entries.'}
			>
				{#if link}
					<span class="mr-1 flex items-center gap-1.5 font-mono text-[10px] uppercase tracking-[0.14em] {linkNeedsReconnect(link) ? 'text-danger' : 'text-accent'}">
						<span class="size-1.5 rounded-full {linkNeedsReconnect(link) ? 'bg-danger' : 'bg-accent'}" aria-hidden="true"></span>
						{linkNeedsReconnect(link) ? 'reconnect' : 'connected'}
					</span>
					{#if linkNeedsReconnect(link)}
						<Button variant="secondary" size="sm" loading={linkBusy === p.id} onclick={() => void connect(p.id)}>Reconnect</Button>
					{:else}
						<Button variant="ghost" size="sm" loading={linkBusy === p.id} onclick={() => void syncNow(p.id)}>
							<RefreshCw class="size-3.5" /> Sync
						</Button>
					{/if}
					<Button variant="ghost" size="sm" disabled={linkBusy === p.id} onclick={() => void disconnect(p.id)}>
						<Unlink class="size-3.5" /> Disconnect
					</Button>
				{:else}
					<Button size="sm" loading={linkBusy === p.id} onclick={() => void connect(p.id)}>Connect</Button>
					{#if pinUrl && !pinOpen}
						<Button variant="ghost" size="sm" onclick={() => (pinOpen = true)}>Use a code</Button>
					{/if}
				{/if}
				{#snippet below()}
					{#if !link && pinOpen && pinUrl}
						<div class="flex flex-wrap items-start gap-2 pt-1">
							<Button variant="secondary" size="sm" onclick={openPin}>1 · Open AniList</Button>
							<div class="min-w-48 flex-1">
								<Input type="password" placeholder="2 · Paste the code" aria-label="AniList code" bind:value={pinCode} error={pinErr || null}
									onkeydown={(e) => e.key === 'Enter' && void submitPin(p.id)} />
							</div>
							<Button size="sm" loading={pinBusy} onclick={() => void submitPin(p.id)}>3 · Link</Button>
						</div>
					{/if}
				{/snippet}
			</SettingRow>
			{#if link}
				<SettingRow
					label="Update {p.name} when I finish an episode"
					hint="Advances entries you already track. Never lowers progress or reopens completed titles."
				>
					<Switch
						checked={link.scrobble}
						disabled={linkBusy === p.id}
						label="Scrobble to {p.name}"
						bare
						onCheckedChange={() => void toggleScrobble(link)}
					/>
				</SettingRow>
			{/if}
		{/each}
	{/if}
</SettingsGroup>
