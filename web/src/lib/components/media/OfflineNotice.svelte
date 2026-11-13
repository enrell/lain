<script lang="ts">
	import Unplug from '@lucide/svelte/icons/unplug';
	import { t } from '$lib/i18n';

	// D-287: what a title page says when its file cannot be reached, in
	// place of a Play that fails: which folder is away and that nothing is
	// lost. Calm on purpose: a disk that is unmounted comes back.
	let {
		reason,
		library = '',
		many = false
	}: {
		reason: string;
		/** The library's name, when known. */
		library?: string;
		/** A title of several files (a show's page). */
		many?: boolean;
	} = $props();

	const where = $derived(library || t('unreachable.itsLibrary'));
</script>

<div class="flex max-w-2xl gap-3 rounded-card border border-line bg-surface px-4 py-3 text-sm" role="status" data-offline-notice>
	<Unplug class="mt-0.5 size-4 shrink-0 text-muted" aria-hidden="true" />
	<div class="min-w-0">
		<p class="font-medium text-foreground">
			{many ? t('unreachable.seriesTitle') : reason === 'library' ? t('unreachable.libraryTitle') : t('unreachable.fileTitle')}
		</p>
		<p class="mt-1 text-muted">
			{reason === 'library' ? t('unreachable.libraryBody', { library: where }) : t('unreachable.fileBody', { library: where })}
		</p>
	</div>
</div>
