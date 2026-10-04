<script lang="ts">
	import { api, type RelationRow } from '$lib/api';
	import { t } from '$lib/i18n';
	import Button from '$lib/components/primitives/Button.svelte';
	import Modal from '$lib/components/primitives/Modal.svelte';
	import Avatar from '$lib/components/profile/Avatar.svelte';
	import { asAvatarUser, socialError, userName } from '$lib/social/format';
	import { toasts } from '$lib/stores/toasts.svelte';
	import { focusSoon } from './focus';
	import Kbd from './Kbd.svelte';
	import { FIELD } from './styles';

	let {
		open = $bindable(false),
		heading,
		onSend
	}: {
		open?: boolean;
		heading: string;
		onSend: (to: string[], message: string) => Promise<void>;
	} = $props();

	let friends = $state<RelationRow[]>([]);
	let ready = $state(false);
	let picked = $state<Set<string>>(new Set());
	let message = $state('');
	let sending = $state(false);
	let list = $state<HTMLDivElement>();

	$effect(() => {
		if (!open) return;
		picked = new Set();
		message = '';
		ready = false;
		api.social
			.friends()
			.then((res) => {
				friends = res.friends;
				ready = true;
				focusSoon(() => list?.querySelector<HTMLElement>('button'));
			})
			.catch((err) => {
				toasts.error(socialError(err, t('social.share.loadFailed')));
				open = false;
			});
	});

	function toggle(id: string): void {
		const next = new Set(picked);
		if (next.has(id)) next.delete(id);
		else next.add(id);
		picked = next;
	}

	async function send(): Promise<void> {
		if (picked.size === 0 || sending) return;
		sending = true;
		try {
			await onSend([...picked], message);
			toasts.success(t('social.share.sent', { count: picked.size }));
			open = false;
		} catch (err) {
			toasts.error(socialError(err, t('social.share.failed')));
		} finally {
			sending = false;
		}
	}

	function onKey(e: KeyboardEvent): void {
		if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) {
			e.preventDefault();
			void send();
		}
	}
</script>

<Modal bind:open title={heading} description={t('social.share.description')}>
	<!-- svelte-ignore a11y_no_static_element_interactions -->
	<div onkeydown={onKey} class="space-y-4">
		{#if !ready}
			<p class="text-sm text-muted">{t('social.loading')}</p>
		{:else if friends.length === 0}
			<p class="text-sm text-muted">{t('social.share.noFriends')}</p>
		{:else}
			<div bind:this={list} class="max-h-64 space-y-1 overflow-y-auto" role="group" aria-label={t('social.share.recipients')}>
				{#each friends as f (f.user.id)}
					<button
						type="button"
						aria-pressed={picked.has(f.user.id)}
						onclick={() => toggle(f.user.id)}
						class="flex w-full items-center gap-3 rounded-md border px-3 py-2 text-start text-sm transition-colors {picked.has(f.user.id)
							? 'border-accent bg-accent/10 text-foreground'
							: 'border-transparent text-muted hover:bg-surface-hover hover:text-foreground'}"
					>
						<Avatar user={asAvatarUser(f.user)} class="size-6" />
						<span class="flex-1 truncate">{userName(f.user)}</span>
						<span class="font-mono text-[10px] uppercase tracking-[0.18em]">{picked.has(f.user.id) ? t('social.share.picked') : ''}</span>
					</button>
				{/each}
			</div>
			<label class="block space-y-1.5">
				<span class="text-xs text-muted">{t('social.share.message')}</span>
				<input
					bind:value={message}
					maxlength="280"
					class={FIELD}
					placeholder={t('social.share.messagePlaceholder')}
				/>
			</label>
		{/if}
	</div>
	{#snippet footer()}
		<Button variant="ghost" onclick={() => (open = false)}>{t('common.cancel')} <Kbd>Esc</Kbd></Button>
		<Button loading={sending} disabled={picked.size === 0} onclick={() => void send()}>
			{t('social.share.send', { count: picked.size })} <Kbd>Ctrl ↵</Kbd>
		</Button>
	{/snippet}
</Modal>
