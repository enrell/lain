<script lang="ts">
	import type { Snippet } from 'svelte';

	let {
		title,
		description,
		icon,
		children
	}: {
		title: string;
		description?: string;
		icon?: Snippet;
		children?: Snippet;
	} = $props();
</script>

<!-- A quiet solid panel: a dashed outline reads as a drop target or a wireframe. -->
<div class="flex flex-col items-center justify-center rounded-card border border-line/50 bg-surface/40 px-6 py-14 text-center">
	<div class="mb-5 flex size-14 items-center justify-center rounded-full bg-surface-active text-foreground">
		{@render icon?.()}
	</div>
	<h2 class="text-base font-semibold text-foreground">{title}</h2>
	{#if description}
		<p class="mt-1 max-w-md text-sm text-muted">{description}</p>
	{/if}
	{#if children}
		<div class="mt-5 flex flex-wrap items-center justify-center gap-2">
			{@render children()}
		</div>
	{/if}
</div>
