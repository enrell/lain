<script lang="ts">
	/*
	 * The social panel of a title page (docs/slices/social.md): ratings,
	 * reviews, comments, share, collections and favorites for the work —
	 * any media kind. Keys: r rate, s share, c collect, f favorite,
	 * m comment.
	 */
	import { api, type Comment, type WorkTarget, type WorkView, targetOf } from '$lib/api';
	import { session } from '$lib/auth/session.svelte';
	import { i18n, t } from '$lib/i18n';
	import Button from '$lib/components/primitives/Button.svelte';
	import { pageKey, socialError, workKey } from '$lib/social/format';
	import { toasts } from '$lib/stores/toasts.svelte';
	import { formatRelative } from '$lib/utilities/format';
	import CollectDialog from './CollectDialog.svelte';
	import Kbd from './Kbd.svelte';
	import RatingDialog from './RatingDialog.svelte';
	import ShareDialog from './ShareDialog.svelte';
	import UserChip from './UserChip.svelte';
	import { FIELD } from './styles';

	let { target }: { target: WorkTarget } = $props();

	let view = $state<WorkView | null>(null);
	let loadError = $state('');
	let favorite = $state(false);
	let favBusy = $state(false);
	let rateOpen = $state(false);
	let shareOpen = $state(false);
	let collectOpen = $state(false);
	let body = $state('');
	let replyTo = $state<Comment | null>(null);
	let posting = $state(false);
	let revealed = $state<Set<string>>(new Set());
	let composer = $state<HTMLTextAreaElement>();

	const targetKey = $derived(JSON.stringify(target));
	const byId = $derived(new Map((view?.comments ?? []).map((c) => [c.id, c])));
	const threads = $derived.by(() => {
		const all = view?.comments ?? [];
		const roots = all.filter((c) => !c.reply_to || !byId.has(c.reply_to));
		return roots.map((root) => ({ root, replies: all.filter((c) => c.reply_to === root.id) }));
	});

	async function load(): Promise<void> {
		loadError = '';
		try {
			view = await api.social.work(target);
			const st = await api.social.settings();
			const key = workKey(view.work);
			favorite = st.favorites.some((f) => workKey(f) === key);
		} catch (err) {
			loadError = socialError(err, t('social.title.loadFailed'));
		}
	}

	$effect(() => {
		void targetKey;
		void load();
	});

	async function saveRating(b: { score: number; review: string; spoiler: boolean }): Promise<void> {
		await api.social.rate(target, b);
		await load();
	}

	async function removeRating(): Promise<void> {
		await api.social.unrate(target);
		await load();
	}

	async function toggleFavorite(): Promise<void> {
		if (!view || favBusy) return;
		favBusy = true;
		try {
			const st = await api.social.settings();
			const key = workKey(view.work);
			const rest = st.favorites.filter((f) => workKey(f) !== key).map(targetOf);
			const next = favorite ? rest : [...rest, targetOf(view.work)];
			const saved = await api.social.updateSettings({ favorites: next });
			favorite = saved.favorites.some((f) => workKey(f) === key);
			toasts.success(favorite ? t('social.title.favorited') : t('social.title.unfavorited'));
		} catch (err) {
			toasts.error(socialError(err, t('social.title.favoriteFailed')));
		} finally {
			favBusy = false;
		}
	}

	async function post(): Promise<void> {
		if (!body.trim() || posting) return;
		posting = true;
		try {
			await api.social.comment(target, body, replyTo?.id);
			body = '';
			replyTo = null;
			await load();
		} catch (err) {
			toasts.error(socialError(err, t('social.comments.failed')));
		} finally {
			posting = false;
		}
	}

	async function removeComment(c: Comment): Promise<void> {
		try {
			await api.social.deleteComment(c.id);
			await load();
		} catch (err) {
			toasts.error(socialError(err, t('social.comments.deleteFailed')));
		}
	}

	function reply(c: Comment): void {
		replyTo = c;
		composer?.focus();
	}

	function onComposerKey(e: KeyboardEvent): void {
		if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) {
			e.preventDefault();
			void post();
		} else if (e.key === 'Escape') {
			replyTo = null;
			composer?.blur();
		}
	}

	function onKey(e: KeyboardEvent): void {
		if (!view) return;
		switch (pageKey(e)) {
			case 'r':
				rateOpen = true;
				break;
			case 's':
				shareOpen = true;
				break;
			case 'c':
				collectOpen = true;
				break;
			case 'f':
				void toggleFavorite();
				break;
			case 'm':
				composer?.focus();
				break;
			default:
				return;
		}
		e.preventDefault();
	}

	const canDelete = (c: Comment) => c.user_id === session.user?.id || session.isAdmin;
</script>

<svelte:window onkeydown={onKey} />

<section class="space-y-6 border-t border-hairline pt-8" aria-labelledby="social-heading">
	<div class="flex flex-wrap items-end justify-between gap-4">
		<div>
			<h2 id="social-heading" class="font-mono text-[10px] font-semibold uppercase tracking-[0.26em] text-muted">{t('social.title.heading')}</h2>
			{#if view && view.scored > 0}
				<p class="mt-2 flex items-baseline gap-3">
					<span class="text-4xl font-semibold tracking-tight text-foreground">{i18n.formatNumber(view.average, { maximumFractionDigits: 1, minimumFractionDigits: 1 })}</span>
					<span class="text-sm text-muted">{t('social.title.scored', { count: view.scored })}</span>
				</p>
			{:else if view}
				<p class="mt-2 text-sm text-muted">{t('social.title.unrated')}</p>
			{/if}
		</div>
		{#if view}
			<div class="flex flex-wrap gap-2">
				<Button size="sm" variant={view.mine ? 'secondary' : 'primary'} onclick={() => (rateOpen = true)}>
					{view.mine?.score ? t('social.title.yourScore', { score: view.mine.score }) : view.mine ? t('social.title.editReview') : t('social.title.rate')}
					<Kbd>R</Kbd>
				</Button>
				<Button size="sm" variant="secondary" onclick={() => (shareOpen = true)}>{t('social.title.share')} <Kbd>S</Kbd></Button>
				<Button size="sm" variant="secondary" onclick={() => (collectOpen = true)}>{t('social.title.collect')} <Kbd>C</Kbd></Button>
				<Button size="sm" variant="ghost" aria-pressed={favorite} loading={favBusy} onclick={() => void toggleFavorite()}>
					{favorite ? t('social.title.favorite') : t('social.title.addFavorite')} <Kbd>F</Kbd>
				</Button>
			</div>
		{/if}
	</div>

	{#if loadError}
		<p class="text-sm text-danger">{loadError}</p>
	{:else if view}
		{#if view.ratings.length > 0}
			<div>
				<h3 class="mb-2 font-mono text-[10px] uppercase tracking-[0.22em] text-muted">{t('social.title.reviews')}</h3>
				<ul>
					{#each view.ratings as r (r.user_id)}
						<li class="border-b border-hairline py-3">
							<div class="flex items-center justify-between gap-3">
								<UserChip user={view.users[r.user_id]} sub={formatRelative(r.updated_at)} />
								{#if r.score}<span class="font-mono text-sm text-foreground">{t('social.score', { score: r.score })}</span>{/if}
							</div>
							{#if r.review}
								{#if r.spoiler && !revealed.has(r.user_id)}
									<button type="button" class="mt-2 font-mono text-[11px] text-accent" onclick={() => (revealed = new Set([...revealed, r.user_id]))}>
										{t('social.title.revealSpoiler')}
									</button>
								{:else}
									<p class="mt-2 whitespace-pre-line text-sm leading-6 text-foreground/90">{r.review}</p>
								{/if}
							{/if}
						</li>
					{/each}
				</ul>
			</div>
		{/if}

		<div>
			<h3 class="mb-2 font-mono text-[10px] uppercase tracking-[0.22em] text-muted">{t('social.comments.heading', { count: view.comments.length })}</h3>
			<ul>
				{#each threads as th (th.root.id)}
					{#each [th.root, ...th.replies] as c (c.id)}
						<li class="border-b border-hairline py-3 {c.reply_to ? 'ps-8' : ''}">
							<div class="flex items-center justify-between gap-3">
								<UserChip user={view.users[c.user_id]} sub={formatRelative(c.created_at)} />
								<div class="flex gap-3 font-mono text-[10px] uppercase tracking-[0.16em]">
									<button type="button" class="text-muted hover:text-foreground" onclick={() => reply(th.root)}>{t('social.comments.reply')}</button>
									{#if canDelete(c)}
										<button type="button" class="text-muted hover:text-danger" onclick={() => void removeComment(c)}>{t('social.comments.delete')}</button>
									{/if}
								</div>
							</div>
							<p class="mt-2 whitespace-pre-line text-sm leading-6 text-foreground/90">{c.body}</p>
						</li>
					{/each}
				{:else}
					<li class="py-3 text-sm text-muted">{t('social.comments.empty')}</li>
				{/each}
			</ul>
			<div class="mt-4 space-y-2">
				{#if replyTo}
					<p class="font-mono text-[10px] text-muted">
						{t('social.comments.replyingTo', { name: view.users[replyTo.user_id]?.display_name || view.users[replyTo.user_id]?.username || '' })}
						<button type="button" class="ms-2 text-accent" onclick={() => (replyTo = null)}>{t('common.cancel')}</button>
					</p>
				{/if}
				<textarea
					bind:this={composer}
					bind:value={body}
					onkeydown={onComposerKey}
					rows="3"
					maxlength="1000"
					class={FIELD}
					aria-label={t('social.comments.label')}
					placeholder={t('social.comments.placeholder')}
				></textarea>
				<div class="flex items-center justify-between">
					<span class="flex items-center gap-1.5 font-mono text-[10px] text-muted"><Kbd>M</Kbd> {t('social.comments.focusHint')}</span>
					<Button size="sm" loading={posting} disabled={!body.trim()} onclick={() => void post()}>
						{t('social.comments.post')} <Kbd>Ctrl ↵</Kbd>
					</Button>
				</div>
			</div>
		</div>
	{/if}
</section>

{#if view}
	<RatingDialog bind:open={rateOpen} title={view.work.title} mine={view.mine} onSave={saveRating} onRemove={removeRating} />
	<ShareDialog
		bind:open={shareOpen}
		heading={t('social.share.workTitle', { title: view.work.title })}
		onSend={async (to, message) => {
			await api.social.share(target, to, message);
		}}
	/>
	<CollectDialog bind:open={collectOpen} work={view.work} />
{/if}
