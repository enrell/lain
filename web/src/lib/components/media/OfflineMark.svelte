<script lang="ts">
	import Unplug from '@lucide/svelte/icons/unplug';
	import { t } from '$lib/i18n';

	// D-287: a calm corner mark on a card whose file cannot be reached
	// (its disk unmounted): said, not alarmed; the title page says why.
	// `compact`: the icon alone, beside another badge on a narrow card.
	let { compact = false, class: className = '' }: { compact?: boolean; class?: string } = $props();
</script>

<span
	class={[
		'pointer-events-none flex items-center gap-1 rounded-full bg-black/65 py-0.5',
		compact ? 'px-1.5' : 'px-2',
		' font-mono text-[10px] font-semibold uppercase tracking-[0.14em] text-white/85 backdrop-blur',
		className
	].join(' ')}
	data-offline-mark
>
	<Unplug class="size-3" aria-hidden="true" />{#if compact}<span class="sr-only">{t('unreachable.mark')}</span>{:else}{t('unreachable.mark')}{/if}
</span>
