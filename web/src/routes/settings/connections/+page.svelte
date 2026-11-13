<script lang="ts">
	import { onMount } from 'svelte';
	import RefreshCw from '@lucide/svelte/icons/refresh-cw';
	import Unlink from '@lucide/svelte/icons/unlink';
	import { goto } from '$app/navigation';
	import { page } from '$app/state';
	import { api, type LinkedAccountView } from '$lib/api';
	import { t } from '$lib/i18n';
	import Button from '$lib/components/primitives/Button.svelte';
	import Input from '$lib/components/primitives/Input.svelte';
	import Spinner from '$lib/components/primitives/Spinner.svelte';
	import Switch from '$lib/components/primitives/Switch.svelte';
	import SettingRow from '$lib/components/settings/SettingRow.svelte';
	import SettingsGroup from '$lib/components/settings/SettingsGroup.svelte';
	import { toasts } from '$lib/stores/toasts.svelte';
	import { errorMessage } from '$lib/utilities/errors';
	import { formatRelative } from '$lib/utilities/format';
	import { statusLabel } from '$lib/tracking/status';

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
			toasts.error(errorMessage(err, t('settings.connectionsPage.loadFailed')));
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
				toasts.error(t('settings.connectionsPage.needsApp'));
			} else {
				toasts.error(
					configured
						? t('settings.connectionsPage.notConfigured', { platform })
						: errorMessage(err, t('settings.connectionsPage.startFailed', { platform }))
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
			pinErr = t('settings.connectionsPage.pasteCode');
			return;
		}
		pinBusy = true;
		pinErr = '';
		try {
			await api.links.code(platform, code);
			toasts.success(t('settings.connectionsPage.connected', { platform }));
			pinOpen = false;
			pinCode = '';
			await loadLinks();
		} catch (err) {
			const code = err instanceof Error && 'code' in err ? (err as { code?: string }).code : '';
			pinErr =
				code === 'invalid-grant'
					? t('settings.connectionsPage.badCode')
					: errorMessage(err, t('settings.connectionsPage.linkFailed'));
		} finally {
			pinBusy = false;
		}
	}

	async function syncNow(platform: string): Promise<void> {
		linkBusy = platform;
		try {
			const res = await api.links.sync(platform);
			toasts.success(t('settings.connectionsPage.synced', { count: res.stats.upserted, removed: res.stats.removed }));
			await loadLinks();
		} catch (err) {
			toasts.error(errorMessage(err, t('settings.connectionsPage.syncFailed')));
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
			toasts.error(errorMessage(err, t('settings.connectionsPage.settingFailed')));
		} finally {
			linkBusy = '';
		}
	}

	async function disconnect(platform: string): Promise<void> {
		linkBusy = platform;
		try {
			await api.links.unlink(platform);
			toasts.success(t('settings.connectionsPage.disconnected', { platform }));
			await loadLinks();
		} catch (err) {
			toasts.error(errorMessage(err, t('settings.connectionsPage.disconnectFailed')));
		} finally {
			linkBusy = '';
		}
	}

	// What lain last sent: the visible proof the switch does something (D-288).
	function lastPush(link: LinkedAccountView): string {
		const p = link.last_push;
		if (!p) return '';
		const when = formatRelative(p.at);
		if (p.error) return t('tracking.account.lastPushFailed', { error: p.error, title: p.title, when });
		if (p.progress) return t('tracking.account.lastPush', { title: p.title, progress: p.progress, when });
		return t('tracking.account.lastPushStatus', { title: p.title, status: statusLabel(p.status ?? '', false), when });
	}

	onMount(() => {
		// The OAuth callback lands back here with a result flag.
		const linked = page.url.searchParams.get('linked');
		const linkError = page.url.searchParams.get('link_error');
		if (linked) toasts.success(t('settings.connectionsPage.connected', { platform: linked }));
		if (linkError) toasts.error(t('settings.connectionsPage.incomplete', { error: linkError }));
		if (linked || linkError) {
			const url = new URL(page.url);
			url.searchParams.delete('linked');
			url.searchParams.delete('link_error');
			void goto(url.pathname + url.search, { replaceState: true });
		}
		void loadLinks();
	});
</script>

<svelte:head><title>{t('settings.connectionsPage.pageTitle')}</title></svelte:head>

<SettingsGroup title={t('settings.connectionsPage.lists')} description={t('settings.groups.connections.lists')}>
	{#if !linksReady}
		<div class="flex justify-center py-8"><Spinner class="size-5 text-muted" /></div>
	{:else}
		{#each platforms as p (p.id)}
			{@const link = linkFor(p.id)}
			<SettingRow
				id={p.id}
				label={p.name}
				hint={link
					? [
							link.remote_username || link.remote_user_id,
							t('settings.connectionsPage.entries', { count: link.entry_count }),
							link.last_sync_at
								? t('settings.connectionsPage.syncedAt', { when: formatRelative(link.last_sync_at) })
								: link.last_sync_error
									? t('settings.connectionsPage.syncError', { error: link.last_sync_error })
									: ''
						]
							.filter(Boolean)
							.join(' · ')
					: t('settings.connectionsPage.importHint')}
			>
				{#if link}
					<span class="mr-1 flex items-center gap-1.5 font-mono text-[10px] uppercase tracking-[0.14em] {linkNeedsReconnect(link) ? 'text-danger' : 'text-accent'}">
						<span class="size-1.5 rounded-full {linkNeedsReconnect(link) ? 'bg-danger' : 'bg-accent'}" aria-hidden="true"></span>
						{linkNeedsReconnect(link) ? t('settings.connectionsPage.stateReconnect') : t('settings.connectionsPage.stateConnected')}
					</span>
					{#if linkNeedsReconnect(link)}
						<Button variant="secondary" size="field" loading={linkBusy === p.id} onclick={() => void connect(p.id)}>{t('settings.connectionsPage.reconnect')}</Button>
					{:else}
						<Button variant="ghost" size="field" loading={linkBusy === p.id} onclick={() => void syncNow(p.id)}>
							<RefreshCw class="size-3.5" /> {t('settings.connectionsPage.sync')}
						</Button>
					{/if}
					<Button variant="ghost" size="field" disabled={linkBusy === p.id} onclick={() => void disconnect(p.id)}>
						<Unlink class="size-3.5" /> {t('settings.connectionsPage.disconnect')}
					</Button>
				{:else}
					<Button size="field" loading={linkBusy === p.id} onclick={() => void connect(p.id)}>{t('settings.connectionsPage.connect')}</Button>
					{#if pinUrl && !pinOpen}
						<Button variant="ghost" size="field" onclick={() => (pinOpen = true)}>{t('settings.connectionsPage.useCode')}</Button>
					{/if}
				{/if}
				{#snippet below()}
					{#if !link && pinOpen && pinUrl}
						<div class="flex flex-wrap items-start gap-2 pt-1">
							<Button variant="secondary" size="field" onclick={openPin}>{t('settings.connectionsPage.step1')}</Button>
							<div class="min-w-48 flex-1">
								<Input type="password" placeholder={t('settings.connectionsPage.step2')} aria-label={t('settings.connectionsPage.codeLabel')} bind:value={pinCode} error={pinErr || null}
									onkeydown={(e) => e.key === 'Enter' && void submitPin(p.id)} />
							</div>
							<Button size="field" loading={pinBusy} onclick={() => void submitPin(p.id)}>{t('settings.connectionsPage.step3')}</Button>
						</div>
					{/if}
				{/snippet}
			</SettingRow>
			{#if link}
				<SettingRow
					label={t('settings.connectionsPage.scrobble', { platform: p.name })}
					hint={[t('settings.connectionsPage.scrobbleHint'), lastPush(link)].filter(Boolean).join(' ')}
				>
					<Switch
						checked={link.scrobble}
						disabled={linkBusy === p.id}
						label={t('settings.connectionsPage.scrobbleTo', { platform: p.name })}
						bare
						onCheckedChange={() => void toggleScrobble(link)}
					/>
				</SettingRow>
			{/if}
		{/each}
	{/if}
</SettingsGroup>
