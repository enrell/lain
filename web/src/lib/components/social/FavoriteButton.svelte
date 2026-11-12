<script lang="ts">
	/*
	 * The heart beside a title's main action: keeps the title on the
	 * account's favourites (the Favorites rail on Home, the profile).
	 * Absent until the social block knows the work, and on a server
	 * without the social module.
	 */
	import Heart from '@lucide/svelte/icons/heart';
	import { t } from '$lib/i18n';
	import { favoriteState } from '$lib/social/favorite.svelte';
	import { socialError } from '$lib/social/format';
	import { toasts } from '$lib/stores/toasts.svelte';

	async function toggle(): Promise<void> {
		try {
			const on = await favoriteState.toggle();
			toasts.success(on ? t('social.title.favorited') : t('social.title.unfavorited'));
		} catch (err) {
			toasts.error(socialError(err, t('social.title.favoriteFailed')));
		}
	}
</script>

{#if favoriteState.ready}
	<!-- The heart alone, as in the app: its name is the tooltip and the accessible name; F does the same.
	     The fill of the title row's other quiet actions (the tonal button); the heart's own fill says "on". -->
	<button
		type="button"
		class="inline-flex size-11 shrink-0 items-center justify-center rounded-md bg-foreground/[0.08] text-foreground transition-colors hover:bg-foreground/[0.14] focus-visible:outline-2 focus-visible:outline-accent disabled:opacity-50"
		aria-pressed={favoriteState.on}
		aria-label={favoriteState.on ? t('social.title.favorite') : t('social.title.addFavorite')}
		title={`${favoriteState.on ? t('social.title.favorite') : t('social.title.addFavorite')} (F)`}
		disabled={favoriteState.busy}
		onclick={() => void toggle()}
	>
		<Heart class={favoriteState.on ? 'size-[18px] fill-accent text-accent' : 'size-[18px]'} aria-hidden="true" />
	</button>
{/if}
