<script lang="ts">
	import ProgressBar from './ProgressBar.svelte';

	let { ratio, label = '' }: { ratio: number; label?: string } = $props();
</script>

<!--
  The bar every card wears, inset and rounded at the picture's foot, on a
  dark fade of its own: straight on the art, a pale cover (a yellow volume,
  a white poster) swallowed both the track and the fill. A card begun by a
  page or a few seconds still shows a sliver of fill.
-->
<div class="pointer-events-none absolute inset-x-0 bottom-0 bg-gradient-to-t from-black/80 via-black/45 to-transparent px-1.5 pb-1.5 pt-6">
	<ProgressBar {ratio} {label} floor={0.04} class="bg-white/30" />
</div>
