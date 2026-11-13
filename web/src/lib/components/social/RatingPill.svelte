<script lang="ts">
	/*
	 * Your score beside a title's main action, as the app shows it: "Rate"
	 * before you gave one, "8" after, with everyone's average when others
	 * scored it too. It opens the social block's rating dialog. It wears the
	 * title row's one quiet fill; the filled star, as the heart's fill, is
	 * what says a score is given.
	 */
	import Star from '@lucide/svelte/icons/star';
	import { i18n, t } from '$lib/i18n';
	import { ratingState } from '$lib/social/favorite.svelte';

	const others = $derived(ratingState.scored - (ratingState.mine > 0 ? 1 : 0));
	const average = $derived(
		others > 0 ? t('social.title.average', { average: i18n.formatNumber(ratingState.average, { minimumFractionDigits: 1, maximumFractionDigits: 1 }) }) : ''
	);
	// Named in full whatever the width shows (under 380 px only the number is drawn),
	// the average included: a name of the score alone left it unsaid.
	const name = $derived(ratingState.mine > 0 ? [t('social.title.yourScore', { score: ratingState.mine }), average].filter(Boolean).join(' · ') : undefined);
</script>

{#if ratingState.ready}
	<button
		type="button"
		class="inline-flex h-11 shrink-0 items-center gap-2 rounded-md bg-foreground/[0.08] px-4 text-sm font-medium text-foreground transition-colors hover:bg-foreground/[0.14]"
		title={t('social.title.scoreHint')}
		aria-label={name}
		onclick={() => ratingState.ask()}
	>
		<Star class={ratingState.mine > 0 ? 'size-4 fill-accent text-accent' : 'size-4'} aria-hidden="true" />
		<!-- A phone under 380 px says the score by the filled star and the number: spelled out,
		     the row's last button (More) fell onto a line of its own. -->
		{#if ratingState.mine > 0}
			<span class="max-[380px]:hidden">{t('social.title.yourScore', { score: ratingState.mine })}</span><span class="hidden tabular-nums max-[380px]:inline">{ratingState.mine}</span>
		{:else}
			{t('social.title.rate')}
		{/if}
		{#if average}<span class="text-muted">· {average}</span>{/if}
	</button>
{/if}
