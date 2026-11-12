<script lang="ts">
	/*
	 * `?`: the keys of the screen in view, then the keys that work on every
	 * page of the shell. A page names its own keys where it handles them
	 * (usePageKeys); they also sit beside their actions where there is room.
	 * Quick find has no button, so this list (and the settings rail) is where
	 * Ctrl+K is learned. The player and the reader, which live outside the
	 * shell, keep their own `?` lists.
	 */
	import { t } from '$lib/i18n';
	import Modal from '$lib/components/primitives/Modal.svelte';
	import { pageKeys, type KeyHint } from '$lib/stores/page-keys.svelte';
	import { shortcutsDialog } from '$lib/stores/shortcuts.svelte';
	import { isTypingTarget } from '$lib/utilities/guards';

	const here = $derived(pageKeys.rows);
	// A page with a filter of its own takes `/`; its row says what the key finds there.
	const everywhere = $derived<KeyHint[]>([
		...(pageKeys.has('/') ? [] : [{ keys: ['/'], label: t('shortcuts.search') }]),
		{ keys: ['Ctrl', 'K'], label: t('shortcuts.quickFind') },
		{ keys: ['←', '→'], label: t('shortcuts.rail') },
		{ keys: ['Esc'], label: t('shortcuts.escape') },
		{ keys: ['?'], label: t('shortcuts.help') }
	]);

	function onKey(e: KeyboardEvent): void {
		if (e.key !== '?' || e.ctrlKey || e.metaKey || e.altKey || e.defaultPrevented) return;
		if (isTypingTarget(e.target) || document.querySelector('[role="dialog"]')) return;
		e.preventDefault();
		shortcutsDialog.open = true;
	}
</script>

<svelte:window onkeydown={onKey} />

<!-- Both lists share one pair of columns (subgrid), so every description starts at one edge. -->
{#snippet list(rows: KeyHint[])}
	<dl class="col-span-2 mt-2 grid grid-cols-subgrid items-center gap-y-2.5 text-sm">
		{#each rows as row (row.keys.join('+') + row.label)}
			<dt class="flex gap-1">{#each row.keys as key, i (i)}<kbd>{key}</kbd>{/each}</dt>
			<dd class="text-muted">{row.label}</dd>
		{/each}
	</dl>
{/snippet}

<Modal bind:open={shortcutsDialog.open} title={t('shortcuts.title')}>
	<div class="grid max-h-[min(32rem,70dvh)] grid-cols-[auto_1fr] gap-x-5 gap-y-5 overflow-y-auto">
		{#if here.length > 0}
			<section class="col-span-2 grid grid-cols-subgrid" aria-labelledby="keys-here">
				<h3 id="keys-here" class="col-span-2 font-mono text-[10px] uppercase tracking-[0.22em] text-muted">{t('shortcuts.here')}</h3>
				{@render list(here)}
			</section>
		{/if}
		<section class="col-span-2 grid grid-cols-subgrid" aria-labelledby={here.length > 0 ? 'keys-everywhere' : undefined}>
			{#if here.length > 0}
				<h3 id="keys-everywhere" class="col-span-2 font-mono text-[10px] uppercase tracking-[0.22em] text-muted">{t('shortcuts.everywhere')}</h3>
			{/if}
			{@render list(everywhere)}
		</section>
	</div>
</Modal>
