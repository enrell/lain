<script lang="ts">
	import { t } from '$lib/i18n';
	/*
	 * The single save model of server settings: changes are staged, then
	 * applied together. Ctrl+S applies and Escape discards from anywhere
	 * on the page, so the bar never has to be reached with a mouse.
	 */
	import Button from '$lib/components/primitives/Button.svelte';

	let {
		count,
		saving = false,
		onapply,
		ondiscard
	}: { count: number; saving?: boolean; onapply: () => void; ondiscard: () => void } = $props();

	function onKey(e: KeyboardEvent): void {
		if (count === 0) return;
		if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 's') {
			e.preventDefault();
			if (!saving) onapply();
		} else if (e.key === 'Escape' && !document.querySelector('[role="dialog"]')) {
			const t = e.target as HTMLElement | null;
			if (t && ['INPUT', 'TEXTAREA', 'SELECT'].includes(t.tagName)) return void t.blur();
			ondiscard();
		}
	}
</script>

<svelte:window onkeydown={onKey} />

{#if count > 0}
	<div
		class="fixed inset-x-0 bottom-16 z-40 border-t border-hairline bg-background/92 backdrop-blur-xl md:bottom-0"
		role="region"
		aria-label={t('settings.staged.region')}
	>
		<div class="mx-auto flex max-w-[1800px] items-center justify-between gap-4 px-5 py-3 sm:px-8 lg:px-10">
			<p class="flex items-center gap-2 font-mono text-[11px] uppercase tracking-[0.14em] text-foreground">
				<span class="size-1.5 rounded-full bg-warning" aria-hidden="true"></span>
				{t('settings.staged.count', { count })}
			</p>
			<div class="flex items-center gap-2">
				<Button variant="ghost" size="sm" disabled={saving} onclick={ondiscard}>
					{t('settings.staged.discard')} <kbd class="ml-1 font-mono text-[10px] text-muted">Esc</kbd>
				</Button>
				<Button size="sm" loading={saving} onclick={onapply}>
					{t('settings.staged.apply')} <kbd class="ml-1 font-mono text-[10px] opacity-70">Ctrl+S</kbd>
				</Button>
			</div>
		</div>
	</div>
{/if}
