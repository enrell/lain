<script lang="ts">
	let { ratio, class: className = '' }: { ratio: number; class?: string } = $props();

	const pct = $derived(Math.max(0, Math.min(1, ratio)) * 100);
</script>

<div class={['h-1 w-full overflow-hidden rounded-full bg-white/15', className].join(' ')}>
	<div class="h-full rounded-full bg-accent transition-[width] duration-200" style={`width:${pct}%`}></div>
</div>
