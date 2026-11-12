<script lang="ts">
	import type { Activity, UserSummary } from '$lib/api';
	import { t } from '$lib/i18n';
	import { isReadable } from '$lib/reader/kinds';
	import { formatRelative } from '$lib/utilities/format';
	import UserChip from './UserChip.svelte';
	import WorkLink from './WorkLink.svelte';

	let { activity, user, showUser = true }: { activity: Activity; user?: UserSummary; showUser?: boolean } =
		$props();

	const reading = $derived(isReadable(activity.work.kind));
	const verb = $derived.by(() => {
		switch (activity.type) {
			case 'progress':
				return reading ? t('social.activity.reading') : t('social.activity.watching');
			case 'completed':
				return reading ? t('social.activity.read') : t('social.activity.watched');
			default:
				return t('social.activity.rated');
		}
	});
	const detail = $derived.by(() => {
		const a = activity;
		if (a.type === 'rated') return a.score ? t('social.score', { score: a.score }) : t('social.activity.reviewed');
		if (reading) {
			if (a.episode) return t('social.unit.chapter', { n: a.episode });
			if (a.season) return t('social.unit.volume', { n: a.season });
			return '';
		}
		// A show without seasons says E5, not S0E5.
		if (a.episode) return a.season ? t('social.unit.episode', { season: a.season, episode: a.episode }) : t('social.unit.episodeOnly', { episode: a.episode });
		return '';
	});
</script>

<!-- A phone gives the person and the time one line and what they did the
     next: in one row the name and verb columns left the title a few letters. -->
<li class="flex flex-wrap items-center gap-x-4 gap-y-1.5 border-b border-hairline py-3">
	{#if showUser}
		<span class="min-w-0 flex-1 sm:w-44 sm:flex-none sm:shrink-0"><UserChip {user} /></span>
	{/if}
	<span class={['flex min-w-0 items-baseline gap-3 sm:flex-1 sm:gap-4', showUser ? 'max-sm:order-last max-sm:basis-full' : 'flex-1'].join(' ')}>
		<span class="shrink-0 font-mono text-[10px] uppercase tracking-[0.2em] text-accent sm:w-24">{verb}</span>
		<span class="min-w-0 flex-1"><WorkLink work={activity.work} {detail} /></span>
	</span>
	<time class="ms-auto shrink-0 font-mono text-[10px] text-muted">{formatRelative(activity.at)}</time>
</li>
