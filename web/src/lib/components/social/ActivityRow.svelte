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
		if (a.episode) return t('social.unit.episode', { season: a.season ?? 0, episode: a.episode });
		return '';
	});
</script>

<li class="flex flex-wrap items-center gap-x-4 gap-y-1 border-b border-hairline py-3">
	{#if showUser}
		<span class="w-44 min-w-0 shrink-0"><UserChip {user} /></span>
	{/if}
	<span class="w-24 shrink-0 font-mono text-[10px] uppercase tracking-[0.2em] text-accent">{verb}</span>
	<span class="min-w-0 flex-1"><WorkLink work={activity.work} {detail} /></span>
	<time class="ms-auto shrink-0 font-mono text-[10px] text-muted">{formatRelative(activity.at)}</time>
</li>
