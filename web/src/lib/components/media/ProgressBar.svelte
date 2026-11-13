<script lang="ts">
	let {
		ratio,
		class: className = '',
		label = '',
		floor = 0
	}: {
		ratio: number;
		class?: string;
		label?: string;
		/** The least fill drawn for a value above zero; the value read out stays exact. */
		floor?: number;
	} = $props();

	const pct = $derived(Math.max(0, Math.min(1, ratio)) * 100);
	const drawn = $derived(pct > 0 ? Math.max(floor * 100, pct) : 0);
</script>

<!-- The bar carries the value itself: its colour and width were the only
     signal, which is not programmatically determinable. -->
<div
	class={['h-1 w-full overflow-hidden rounded-full bg-white/15', className].join(' ')}
	role="progressbar"
	aria-valuemin="0"
	aria-valuemax="100"
	aria-valuenow={Math.round(pct)}
	aria-label={label || undefined}
>
	<div class="h-full rounded-full bg-accent transition-[width] duration-200" style={`width:${drawn}%`}></div>
</div>
