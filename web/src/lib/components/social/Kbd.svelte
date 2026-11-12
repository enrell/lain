<script lang="ts">
	let { children }: { children: import('svelte').Snippet } = $props();
</script>

<kbd class="text-muted">{@render children()}</kbd>
