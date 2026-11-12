<script lang="ts">
	/*
	 * Manga adaptation of ReaderBase (D-085): right-to-left by default,
	 * two-page spreads on wide screens with the cover standing alone,
	 * a per-title direction override (a few titles print left to right)
	 * and a webtoon strip for long-scroll releases.
	 */
	import Columns2 from '@lucide/svelte/icons/columns-2';
	import GalleryVertical from '@lucide/svelte/icons/gallery-vertical';
	import type { CatalogItem, ReaderView } from '$lib/api/types';
	import { normalizeSeriesTitle } from '$lib/utilities/grouping';
	import { readingLabel } from '$lib/reader/kinds';
	import { MANGA_PROFILE } from '$lib/reader/profile';
	import { ReaderSettings } from '$lib/reader/settings.svelte';
	import ReaderBase from './ReaderBase.svelte';
	import ReaderGroup from './ReaderGroup.svelte';
	import ReaderToggle from './ReaderToggle.svelte';
	import { t } from '$lib/i18n';

	let {
		item,
		view,
		startAt,
		backHref,
		prev = null,
		next = null,
		onPage
	}: {
		item: CatalogItem;
		view: ReaderView;
		startAt: number;
		backHref: string;
		prev?: { href: string; label: string } | null;
		next?: { href: string; label: string } | null;
		onPage?: (page: number, total: number) => void;
	} = $props();

	// svelte-ignore state_referenced_locally
	const settings = new ReaderSettings(MANGA_PROFILE, normalizeSeriesTitle(item.title), view.direction);
</script>

<ReaderBase
	{item}
	{view}
	{startAt}
	{settings}
	profile={MANGA_PROFILE}
	label={readingLabel(item)}
	{backHref}
	{prev}
	{next}
	{onPage}
>
	{#snippet toolbar()}
		{#if settings.mode === 'paged'}
			<!-- Direction, then how pages are laid out, then the strip: three questions, each in
			     its own track. Both directions are named, so left to right is a lit "LTR", not
			     only an unlit "RTL". -->
			<ReaderGroup label={t('reader.toolbar.direction')}>
				<ReaderToggle
					label={t('reader.toolbar.ltrLabel')}
					active={settings.direction === 'ltr'}
					onclick={() => settings.setDirection('ltr')}
				>
					{t('reader.toolbar.ltr')}
				</ReaderToggle>
				<ReaderToggle
					label={t('reader.toolbar.rtlLabel')}
					active={settings.direction === 'rtl'}
					onclick={() => settings.setDirection('rtl')}
				>
					{t('reader.toolbar.rtl')}
				</ReaderToggle>
			</ReaderGroup>
			<!-- Spreads exist only on a screen wider than tall, so an upright one does not offer them. -->
			<ReaderGroup label={t('reader.toolbar.layout')} class="portrait:hidden">
				<ReaderToggle
					label={t('reader.toolbar.spreadsManga')}
					active={settings.dual}
					onclick={() => settings.toggleDual()}
				>
					<Columns2 class="size-4" aria-hidden="true" />
					<span class="hidden max-md:inline xl:inline">{t('reader.toolbar.spread')}</span>
				</ReaderToggle>
				{#if settings.dual}
					<!-- An option of the spread, drawn as one: a check box, not a second
					     filled mode, and always named, since a lone box says nothing. -->
					<ReaderToggle
						label={t('reader.toolbar.coverLabel')}
						check
						active={settings.coverAlone}
						onclick={() => settings.toggleCoverAlone()}
					>
						<span class="xl:hidden">{t('reader.toolbar.cover')}</span><span class="hidden xl:inline">{t('reader.toolbar.coverAlone')}</span>
					</ReaderToggle>
				{/if}
			</ReaderGroup>
		{/if}
		<ReaderGroup label={t('reader.toolbar.strip')}>
			<ReaderToggle
				label={t('reader.toolbar.webtoonLabel')}
				active={settings.mode === 'strip'}
				onclick={() => settings.setMode(settings.mode === 'strip' ? 'paged' : 'strip')}
			>
				<GalleryVertical class="size-4" aria-hidden="true" />
				<span class="hidden max-md:inline xl:inline">{t('reader.toolbar.webtoon')}</span>
			</ReaderToggle>
		</ReaderGroup>
	{/snippet}
</ReaderBase>
