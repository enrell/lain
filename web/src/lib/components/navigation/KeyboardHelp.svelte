<script lang="ts">
	/*
	 * `?`: the keys that work on every page of the shell. Quick find has no
	 * button any more, so this list (and the settings rail) is where Ctrl+K
	 * is learned. A page's own keys sit beside its actions; the player and
	 * the reader, which live outside the shell, keep their own `?` lists.
	 */
	import { t } from '$lib/i18n';
	import Modal from '$lib/components/primitives/Modal.svelte';
	import { shortcutsDialog } from '$lib/stores/shortcuts.svelte';
	import { isTypingTarget } from '$lib/utilities/guards';

	const rows = $derived<[string[], string][]>([
		[['/'], t('shortcuts.search')],
		[['Ctrl', 'K'], t('shortcuts.quickFind')],
		[['Esc'], t('shortcuts.escape')],
		[['?'], t('shortcuts.help')]
	]);

	function onKey(e: KeyboardEvent): void {
		if (e.key !== '?' || e.ctrlKey || e.metaKey || e.altKey || e.defaultPrevented) return;
		if (isTypingTarget(e.target) || document.querySelector('[role="dialog"]')) return;
		e.preventDefault();
		shortcutsDialog.open = true;
	}
</script>

<svelte:window onkeydown={onKey} />

<Modal bind:open={shortcutsDialog.open} title={t('shortcuts.title')}>
	<dl class="mt-2 grid grid-cols-[auto_1fr] items-center gap-x-5 gap-y-2.5 text-sm">
		{#each rows as [keys, what] (what)}
			<dt class="flex gap-1">{#each keys as key (key)}<kbd>{key}</kbd>{/each}</dt>
			<dd class="text-muted">{what}</dd>
		{/each}
	</dl>
</Modal>
