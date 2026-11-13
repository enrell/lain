<script lang="ts">
	/*
	 * "Set up with a coding agent" (D-290, D-291): mints an integration key
	 * that only manages indexers and expires, and shows a prompt to paste
	 * into Claude Code, opencode or a DeepSeek harness. One prompt fits all
	 * three: each runs curl in a shell, and the rules (ask before adding,
	 * the key only in a header) are the same. The prompt points at the
	 * server's own guide (/api/agent/indexers.md) for the exact calls.
	 *
	 * The key is shown only here and only until the dialog closes; the
	 * dialog lists the agent keys still alive and revokes any of them.
	 * Keys: Enter creates, c copies the prompt, r revokes the new key.
	 */
	import Check from '@lucide/svelte/icons/check';
	import Clipboard from '@lucide/svelte/icons/clipboard';
	import { api } from '$lib/api';
	import type { ApiKey } from '$lib/api/list';
	import Button from '$lib/components/primitives/Button.svelte';
	import Modal from '$lib/components/primitives/Modal.svelte';
	import Segmented from '$lib/components/primitives/Segmented.svelte';
	import { focusSoon } from '$lib/components/social/focus';
	import { t } from '$lib/i18n';
	import { toasts } from '$lib/stores/toasts.svelte';
	import { errorMessage } from '$lib/utilities/errors';
	import { formatRelative } from '$lib/utilities/format';
	import { isTypingTarget } from '$lib/utilities/guards';

	let { open = $bindable(false) }: { open?: boolean } = $props();

	const TTLS = ['900', '3600', '14400', '86400'] as const;
	type Ttl = (typeof TTLS)[number];
	let ttl = $state<Ttl>('3600');
	let creating = $state(false);
	let fresh = $state<ApiKey | null>(null);
	let keys = $state<ApiKey[]>([]);
	let copied = $state(false);
	// Re-renders "expires in …" while the dialog is open.
	let now = $state(Date.now());

	const live = $derived(keys.filter((k) => k.scopes?.includes('indexers') && (k.expires_at ?? 0) * 1000 > now));
	const server = typeof window === 'undefined' ? '' : window.location.origin;
	const prompt = $derived(
		fresh?.key
			? t('acquire.agent.prompt', {
					server,
					key: fresh.key,
					guide: `${server}/api/agent/indexers.md`,
					expires: new Date((fresh.expires_at ?? 0) * 1000).toISOString()
				})
			: ''
	);

	let wasOpen = false;
	$effect(() => {
		if (open && !wasOpen) {
			fresh = null;
			copied = false;
			void load();
		}
		wasOpen = open;
	});

	$effect(() => {
		if (!open) return;
		const timer = setInterval(() => (now = Date.now()), 30_000);
		return () => clearInterval(timer);
	});

	async function load(): Promise<void> {
		try {
			keys = (await api.apiKeys.list()).integrations;
		} catch (err) {
			toasts.error(errorMessage(err, t('settings.integrationsPage.keysLoadFailed')));
		}
	}

	async function create(e: SubmitEvent): Promise<void> {
		e.preventDefault();
		creating = true;
		try {
			fresh = await api.apiKeys.create({ name: t('acquire.agent.keyName'), scopes: ['indexers'], ttl_seconds: Number(ttl) });
			copied = false;
			now = Date.now();
			await load();
			focusSoon(() => document.getElementById('agent-copy'));
		} catch (err) {
			toasts.error(errorMessage(err, t('settings.integrationsPage.createFailed')));
		} finally {
			creating = false;
		}
	}

	async function copy(): Promise<void> {
		if (!prompt) return;
		try {
			await navigator.clipboard.writeText(prompt);
			copied = true;
		} catch {
			toasts.error(t('settings.integrationsPage.copyFailed'));
		}
	}

	async function revoke(key: ApiKey): Promise<void> {
		try {
			await api.apiKeys.remove(key.id);
			if (fresh?.id === key.id) fresh = null;
			keys = keys.filter((k) => k.id !== key.id);
			toasts.success(t('acquire.agent.revoked'));
		} catch (err) {
			toasts.error(errorMessage(err, t('settings.integrationsPage.revokeFailed')));
		}
	}

	function onKey(e: KeyboardEvent): void {
		if (!open || !fresh || e.ctrlKey || e.metaKey || e.altKey || isTypingTarget(e.target)) return;
		if (e.key === 'c') {
			e.preventDefault();
			void copy();
		} else if (e.key === 'r') {
			e.preventDefault();
			void revoke(fresh);
		}
	}

	const ttlLabel = (v: Ttl): string =>
		v === '900' ? t('acquire.agent.ttl15m') : v === '3600' ? t('acquire.agent.ttl1h') : v === '14400' ? t('acquire.agent.ttl4h') : t('acquire.agent.ttl1d');
	const label = 'font-mono text-[10px] uppercase tracking-[0.2em] text-muted';
</script>

<svelte:window onkeydown={onKey} />

<Modal bind:open title={t('acquire.agent.title')} description={t('acquire.agent.description')}>
	{#if !fresh}
		<form id="agent-form" class="space-y-3" onsubmit={create}>
			<div class="space-y-1.5">
				<span class="block text-xs text-muted">{t('acquire.agent.ttl')}</span>
				<Segmented label={t('acquire.agent.ttl')} value={ttl} onchange={(v) => (ttl = v)} options={TTLS.map((v) => ({ value: v, label: ttlLabel(v) }))} />
			</div>
			<ul class="space-y-1 text-xs text-muted">
				<li>{t('acquire.agent.can')}</li>
				<li>{t('acquire.agent.cannot')}</li>
				<li>{t('acquire.agent.asks')}</li>
			</ul>
		</form>
	{:else}
		<div class="space-y-2">
			<p class="text-sm text-foreground">{t('acquire.agent.ready', { when: formatRelative(fresh.expires_at) })}</p>
			<textarea readonly rows="12" aria-label={t('acquire.agent.promptLabel')} class="w-full resize-y rounded-md bg-field p-3 font-mono text-[11px] leading-relaxed text-foreground/90" value={prompt}></textarea>
			<p class="text-xs text-muted">{t('acquire.agent.harnesses')}</p>
		</div>
	{/if}
	{#if live.length > 0}
		<div class="mt-4 space-y-1">
			<p class={label}>{t('acquire.agent.liveKeys', { count: live.length })}</p>
			<ul>
				{#each live as k (k.id)}
					<li class="flex min-h-9 items-center gap-3 text-xs">
						<span class="min-w-0 flex-1 truncate text-foreground/85">{k.name} · <span class="font-mono text-muted">{k.id}</span></span>
						<span class="shrink-0 text-muted">{t('acquire.agent.expires', { when: formatRelative(k.expires_at) })}</span>
						<Button size="field" variant="ghost" class="hover:text-danger!" onclick={() => void revoke(k)} aria-label={t('settings.integrationsPage.revokeFor', { name: k.id })}>
							{t('settings.groups.integrations.revoke')}{#if k.id === fresh?.id} <kbd class="ms-1 font-mono text-[10px] text-muted">r</kbd>{/if}
						</Button>
					</li>
				{/each}
			</ul>
		</div>
	{/if}
	{#snippet footer()}
		<Button variant="ghost" onclick={() => (open = false)}>{t('common.close')} <kbd class="ms-1 font-mono text-[10px] text-muted">Esc</kbd></Button>
		{#if !fresh}
			<Button data-autofocus type="submit" form="agent-form" loading={creating}>{t('acquire.agent.create')} <kbd class="ms-1 font-mono text-[10px] opacity-70">Enter</kbd></Button>
		{:else}
			<Button id="agent-copy" onclick={() => void copy()}>
				{#if copied}<Check class="size-4" /> {t('settings.integrationsPage.copied')}{:else}<Clipboard class="size-4" /> {t('acquire.agent.copy')}{/if}
				<kbd class="ms-1 font-mono text-[10px] opacity-70">c</kbd>
			</Button>
		{/if}
	{/snippet}
</Modal>
