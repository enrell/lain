<script lang="ts">
	/*
	 * PDF adaptation of ReaderBase (D-306): pages rendered by the server's
	 * own PDF library at the size they are shown, read like a comic: one
	 * page at a time, fit and zoom, spreads on request, a continuous scroll.
	 */
	import Columns2 from '@lucide/svelte/icons/columns-2';
	import GalleryVertical from '@lucide/svelte/icons/gallery-vertical';
	import ZoomIn from '@lucide/svelte/icons/zoom-in';
	import ZoomOut from '@lucide/svelte/icons/zoom-out';
	import type { CatalogItem, ReaderView } from '$lib/api/types';
	import { PDF_PROFILE } from '$lib/reader/profile';
	import { ReaderSettings } from '$lib/reader/settings.svelte';
	import ReaderBase from './ReaderBase.svelte';
	import ReaderGroup from './ReaderGroup.svelte';
	import ReaderToggle from './ReaderToggle.svelte';
	import { t } from '$lib/i18n';

	let {
		item,
		view,
		title,
		startAt,
		backHref,
		onPage
	}: {
		item: CatalogItem;
		view: ReaderView;
		/** The book's own title when it has one. */
		title: string;
		startAt: number;
		backHref: string;
		onPage?: (page: number, total: number) => void;
	} = $props();

	// svelte-ignore state_referenced_locally
	const settings = new ReaderSettings(PDF_PROFILE, `pdf:${item.id}`, 'ltr');
</script>

<ReaderBase {item} {view} {startAt} {settings} profile={PDF_PROFILE} label={title || item.title} {backHref} {onPage}>
	{#snippet toolbar()}
		{#if settings.mode === 'paged'}
			<ReaderGroup label={t('reader.toolbar.size')}>
				<ReaderToggle
					label={settings.fit === 'contain' ? t('reader.toolbar.fitPage') : t('reader.toolbar.fitWidth')}
					onclick={() => settings.setFit(settings.fit === 'contain' ? 'width' : 'contain')}
				>
					{settings.fit === 'contain' ? t('reader.toolbar.fit') : t('reader.toolbar.width')}
				</ReaderToggle>
				<ReaderToggle label={t('reader.toolbar.zoomOut')} disabled={settings.zoom <= 1} onclick={() => settings.setZoom(settings.zoom - 0.25)}>
					<ZoomOut class="size-4" aria-hidden="true" />
				</ReaderToggle>
				<span class="w-10 text-center font-mono text-[11px] tabular-nums text-muted">{t('reader.toolbar.zoomLevel', { percent: Math.round(settings.zoom * 100) })}</span>
				<ReaderToggle label={t('reader.toolbar.zoomIn')} disabled={settings.zoom >= 3} onclick={() => settings.setZoom(settings.zoom + 0.25)}>
					<ZoomIn class="size-4" aria-hidden="true" />
				</ReaderToggle>
			</ReaderGroup>
			<ReaderGroup label={t('reader.toolbar.layout')} class="portrait:hidden">
				<ReaderToggle label={t('reader.toolbar.spreadsComic')} active={settings.dual} onclick={() => settings.toggleDual()}>
					<Columns2 class="size-4" aria-hidden="true" />
					<span class="hidden max-md:inline xl:inline">{t('reader.toolbar.spread')}</span>
				</ReaderToggle>
			</ReaderGroup>
		{/if}
		<ReaderGroup label={t('reader.toolbar.strip')}>
			<ReaderToggle label={t('reader.toolbar.scrollLabel')} active={settings.mode === 'strip'} onclick={() => settings.setMode(settings.mode === 'strip' ? 'paged' : 'strip')}>
				<GalleryVertical class="size-4" aria-hidden="true" />
				<span class="hidden max-md:inline xl:inline">{t('reader.toolbar.scroll')}</span>
			</ReaderToggle>
		</ReaderGroup>
	{/snippet}
</ReaderBase>
