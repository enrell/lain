<script lang="ts">
	let { children }: { children: import('svelte').Snippet } = $props();
</script>

<kbd class="inline-flex min-w-4 items-center justify-center rounded border border-hairline px-1 font-mono text-[10px] leading-4 text-muted">{@render children()}</kbd>
