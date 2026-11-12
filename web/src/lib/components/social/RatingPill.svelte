<script lang="ts">
	/*
	 * Your score beside a title's main action, as the app shows it: "Rate"
	 * before you gave one, "8" after, with everyone's average when others
	 * scored it too. It opens the social block's rating dialog. It wears the
	 * title row's one quiet fill; the filled star, as the heart's fill, is
	 * what says a score is given.
	 */
	import Star from '@lucide/svelte/icons/star';
	import { ratingState } from '$lib/social/favorite.svelte';

	const others = $derived(ratingState.scored - (ratingState.mine > 0 ? 1 : 0));
</script>

{#if ratingState.ready}
	<button
		type="button"
		class="inline-flex h-11 shrink-0 items-center gap-2 rounded-md bg-foreground/[0.08] px-4 text-sm font-medium text-foreground transition-colors hover:bg-foreground/[0.14]"
		title="Your score (R)"
		onclick={() => ratingState.ask()}
	>
		<Star class={ratingState.mine > 0 ? 'size-4 fill-accent text-accent' : 'size-4'} aria-hidden="true" />
		{ratingState.mine > 0 ? `Your score: ${ratingState.mine}` : 'Rate'}
		{#if others > 0}<span class="text-muted">· {ratingState.average.toFixed(1)} avg</span>{/if}
	</button>
{/if}
